use super::*;

impl TerminalView {
    pub(super) fn handle_ai_controller_event(
        &mut self,
        _: ModelHandle<BlocklistAIController>,
        event: &BlocklistAIControllerEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if matches!(
            event,
            BlocklistAIControllerEvent::FreeTierLimitCheckTriggered
        ) {
            ctx.emit(Event::FreeTierLimitCheckTriggered);
        }
        if let BlocklistAIControllerEvent::SentRequest { model_id, .. } = event {
            self.maybe_insert_aws_bedrock_login_banner(model_id, ctx);
        }
        if let BlocklistAIControllerEvent::ExecuteLocalHarnessCommand { command } = event {
            self.execute_command_or_set_pending(command, ctx);
        }
        if let BlocklistAIControllerEvent::FinishedReceivingOutput {
            conversation_id, ..
        } = event
        {
            // If the conversation still has a subagent in flight (e.g. a CLI
            // subagent managing a long-running command), the response stream
            // that just ended belongs to the subagent or to the main agent
            // handing off to it — not the end of the overall turn. Defer
            // conversation-finished side effects (e.g. firing a queued `/queue`
            // prompt) until the entire turn is actually done.
            let has_active_subagent = || {
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(conversation_id)
                    .is_some_and(|c| c.has_active_subagent())
            };

            let mut finish_reason: Option<FinishReason> = None;
            if let Some(active_ai_block) = self.active_ai_block(ctx) {
                // Focus the block so that the user can interact
                // with any blocking actions (if any).
                self.focus_ai_block_if_self_focused(active_ai_block, ctx);

                // A new exchange is already active, so callbacks for the
                // just-finished exchange will be skipped. Clear any pending
                // user query now to prevent its callback from firing when
                // the new exchange eventually completes.
                //
                // However, if the active block belongs to the same conversation
                // that has the queued prompt (e.g. a blocked tool-call approval),
                // keep the pending query — the conversation hasn't truly moved on.
                let active_block_conversation_id = active_ai_block.as_ref(ctx).conversation_id();
                let pending_query_conversation_id = self.pending_user_query_conversation_id();
                let is_same_conversation = pending_query_conversation_id
                    .is_some_and(|id| id == active_block_conversation_id);
                if self.pending_user_query_view_id.is_some() && !is_same_conversation {
                    self.remove_pending_user_query_block(ctx);
                }
            } else if !has_active_subagent() {
                if let Some(last_ai_block) = self.last_ai_block() {
                    finish_reason = last_ai_block.as_ref(ctx).finish_reason();
                }
            }

            if let Some(reason) = finish_reason {
                self.handle_finished_conversation(*conversation_id, reason, ctx);
            }

            // If the most recent action in the current interaction turn created or updated a plan
            // document, show an "Execute this plan" prompt suggestion.
            let mut should_show_execute_plan_suggestion = false;
            for view in self.rich_content_views.iter().rev() {
                if let Some(ai_metadata) = view.ai_block_metadata() {
                    let block = ai_metadata.ai_block_handle.as_ref(ctx);

                    if let Some(output) = block.output_status(ctx).output_to_render() {
                        if let Some(most_recent_action) = output.get().actions().last() {
                            should_show_execute_plan_suggestion = matches!(
                                &most_recent_action.action,
                                AIAgentActionType::CreateDocuments(_)
                                    | AIAgentActionType::EditDocuments(_)
                            );
                            break;
                        }
                    }

                    if block.has_user_input(ctx) {
                        // We reached the start of the current interaction turn.
                        break;
                    }
                }
            }

            if should_show_execute_plan_suggestion
                && !FeatureFlag::PromptSuggestionsViaMAA.is_enabled()
            {
                if let Some(block) = self.last_ai_block() {
                    let block_id = BlockId::from(block.id().to_string());
                    let suggestion = AgentModePromptSuggestion::Success(PromptSuggestion {
                        id: Uuid::new_v4().to_string(),
                        label: Some("Execute this plan".to_string()),
                        prompt: "Execute this plan".to_string(),
                        coding_query_context: None,
                        static_prompt_suggestion_name: Some("EXECUTE_CREATED_PLAN".to_string()),
                        should_start_new_conversation: false,
                    });

                    self.on_legacy_prompt_suggestion_generated(
                        suggestion,
                        block_id,
                        "".to_string(),
                        0,
                        ctx,
                    );
                }
            }

            self.update_input_prompt_suggestions_banner_state(ctx);
            ctx.notify();
        }
    }

    /// Drains one prompt from the queued-query singleton for `conversation_id` when that
    /// conversation finishes.
    pub(super) fn drain_queued_prompts(
        &mut self,
        conversation_id: AIConversationId,
        finish_reason: FinishReason,
        ctx: &mut ViewContext<Self>,
    ) {
        match finish_reason {
            FinishReason::Complete => {
                let input_is_empty = self.input.as_ref(ctx).buffer_text(ctx).is_empty();
                let first_row_is_in_edit_mode =
                    QueuedQueryModel::as_ref(ctx).first_row_is_in_edit_mode(conversation_id);
                if first_row_is_in_edit_mode && !input_is_empty {
                    return;
                }

                let action = QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
                    model.pop_for_autofire(conversation_id, ctx)
                });
                match action {
                    Some(AutofireAction::Submit { text }) => {
                        self.input.update(ctx, |input, ctx| {
                            input.submit_queued_prompt(text, ctx);
                        });
                    }
                    Some(AutofireAction::PopFromEditMode { text }) => {
                        self.input.update(ctx, |input, ctx| {
                            if input.buffer_text(ctx).is_empty() {
                                input.replace_buffer_content(&text, ctx);
                                input.focus_input_box(ctx);
                            }
                        });
                    }
                    None => {}
                }
            }
            FinishReason::Error
            | FinishReason::Cancelled
            | FinishReason::CancelledDuringRequestedCommandExecution => {
                // Only restore the head into the input when the user is
                // currently viewing this conversation in agent view. Cancels
                // triggered by exiting the agent view leave `agent_view_state`
                // Inactive by the time the cancel fires, so the head stays in
                // the queue and re-entering the agent view shows the same
                // queue the user left.
                let is_active_in_agent_view = self
                    .agent_view_controller
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id()
                    == Some(conversation_id);
                if !is_active_in_agent_view {
                    return;
                }

                let input_is_empty = self.input.as_ref(ctx).buffer_text(ctx).is_empty();
                if !input_is_empty {
                    return;
                }

                let popped = QueuedQueryModel::handle(ctx)
                    .update(ctx, |model, ctx| model.pop_front(conversation_id, ctx));
                if let Some(query) = popped {
                    self.input.update(ctx, |input, ctx| {
                        input.replace_buffer_content(query.text(), ctx);
                    });
                }
            }
        }
    }

    pub(super) fn handle_legacy_passive_suggestions_event(
        &mut self,
        _: ModelHandle<LegacyPassiveSuggestionsModel>,
        event: &LegacyPassiveSuggestionsEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            LegacyPassiveSuggestionsEvent::PromptSuggestionsGenerated {
                prompt_suggestion,
                block_id,
                command,
                request_duration_ms,
            } => {
                self.on_legacy_prompt_suggestion_generated(
                    prompt_suggestion.clone(),
                    block_id.clone(),
                    command.clone(),
                    *request_duration_ms,
                    ctx,
                );
            }
            LegacyPassiveSuggestionsEvent::PassiveCodeDiffRequestStarted {
                prompt_suggestion_id,
                code_exchange_id,
                block_id,
            } => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::SuggestedCodeDiffBannerShown {
                        prompt_suggestion_id: prompt_suggestion_id.clone(),
                        code_exchange_id: *code_exchange_id,
                        block_id: Some(block_id.to_string()),
                        request_duration_ms: 0,
                        server_request_token: None,
                    },
                    ctx
                );
            }
            LegacyPassiveSuggestionsEvent::PassiveCodeDiffFailed { reason } => {
                self.try_clear_prompt_suggestions_banner_code_state(*reason, ctx);
            }
        }
    }

    pub(super) fn build_agent_todos_popup(
        ai_context_model: ModelHandle<BlocklistAIContextModel>,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<AgentTodosPopupView> {
        let terminal_view_id = ctx.view_id();
        let agent_todos_popup = ctx.add_typed_action_view(move |ctx| {
            AgentTodosPopupView::new(terminal_view_id, ai_context_model, ctx)
        });

        ctx.subscribe_to_view(&agent_todos_popup, |me, _, event, ctx| {
            me.handle_agent_todos_popup_event(event, ctx);
        });

        agent_todos_popup
    }

    pub fn attach_path_as_context(&mut self, path: &Path, ctx: &mut ViewContext<Self>) {
        // If a CLI agent is running, write the path directly to the PTY.
        if self.active_cli_agent(ctx).is_some() {
            let content = path.to_string_lossy().to_string();
            self.write_to_pty(content.into_bytes(), ctx);
            self.focus_terminal(ctx);
            return;
        }

        self.input.update(ctx, |input, ctx| {
            let content = path.to_string_lossy();
            input.append_to_buffer(content.as_ref(), ctx);
            ctx.notify();
        });
    }

    pub fn attach_plan_as_context(
        &mut self,
        ai_document_id: AIDocumentId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.input.update(ctx, |input, ctx| {
            let content = format!("<plan:{ai_document_id}>");
            input.append_to_buffer(content.as_str(), ctx);
            ctx.notify();
        });
    }

    /// Marks this view as hosting a split-off child; pane header switches
    /// from the pill bar to a parent→child breadcrumb row.
    pub fn mark_as_orchestration_split_off(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.is_orchestration_split_off {
            self.is_orchestration_split_off = true;
            ctx.notify();
        }
    }

    /// Clears the split-off marker so the pill bar renders again.
    pub fn clear_orchestration_split_off(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_orchestration_split_off {
            self.is_orchestration_split_off = false;
            ctx.notify();
        }
    }

    /// Whether this view renders the breadcrumb row instead of the pill bar.
    pub fn is_orchestration_split_off(&self) -> bool {
        self.is_orchestration_split_off
    }

    /// Returns true if the given conversation is currently selected in this terminal.
    pub fn is_conversation_selected(
        &self,
        conversation_id: &AIConversationId,
        ctx: &AppContext,
    ) -> bool {
        self.ai_context_model
            .as_ref(ctx)
            .selected_conversation_id(ctx)
            .map(|id| id == *conversation_id)
            .unwrap_or(false)
    }

    fn handle_agent_todos_popup_event(
        &mut self,
        event: &AgentTodosPopupEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            AgentTodosPopupEvent::Close => {
                self.is_todo_popup_visible = false;
                ctx.focus_self();
                ctx.notify();
            }
        }
    }

    pub(super) fn handle_ai_context_model_event(
        &mut self,
        context_model: ModelHandle<BlocklistAIContextModel>,
        event: &BlocklistAIContextEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            BlocklistAIContextEvent::UpdatedPendingContext {
                previous_block_ids,
                requires_block_resync,
                requires_text_resync,
            } => {
                let pending_context_block_ids =
                    context_model.as_ref(ctx).pending_context_block_ids();
                let pending_context_selected_text = context_model
                    .as_ref(ctx)
                    .pending_context_selected_text()
                    .cloned();

                self.ai_render_context.borrow_mut().block_ids.insert(
                    AIContextInclusionState::Pending,
                    pending_context_block_ids.clone(),
                );
                self.ai_render_context
                    .borrow_mut()
                    .has_pending_context_selected_text = pending_context_selected_text.is_some();
                if let Some(conversation_id) = self
                    .agent_view_controller
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id()
                {
                    // Associate newly added blocks with the conversation
                    let added_block_ids = pending_context_block_ids
                        .difference(previous_block_ids)
                        .collect_vec();
                    if !added_block_ids.is_empty() {
                        let associated_blocks = self
                            .model
                            .lock()
                            .block_list_mut()
                            .associate_blocks_with_conversation(
                                added_block_ids.into_iter(),
                                conversation_id,
                            );

                        // Persist the updated visibility for each block
                        if let Some(sender) = GlobalResourceHandlesProvider::as_ref(ctx)
                            .get()
                            .model_event_sender
                            .as_ref()
                        {
                            for (block_id, agent_view_visibility) in associated_blocks {
                                if let Err(e) = sender.send(
                                    persistence::ModelEvent::UpdateBlockAgentViewVisibility {
                                        block_id: block_id.to_string(),
                                        agent_view_visibility: agent_view_visibility.into(),
                                    },
                                ) {
                                    log::error!(
                                        "Error sending UpdateBlockAgentViewVisibility event: {e:?}"
                                    );
                                }
                            }
                        }
                    }

                    // Dissociate removed blocks from the conversation
                    let removed_block_ids = previous_block_ids
                        .difference(pending_context_block_ids)
                        .collect_vec();

                    if !removed_block_ids.is_empty() {
                        let dissociated_blocks = self
                            .model
                            .lock()
                            .block_list_mut()
                            .remove_pending_context_assocation_for_blocks(
                                removed_block_ids.into_iter(),
                                conversation_id,
                            );

                        if let Some(sender) = GlobalResourceHandlesProvider::as_ref(ctx)
                            .get()
                            .model_event_sender
                            .as_ref()
                        {
                            for (block_id, agent_view_visibility) in dissociated_blocks {
                                if let Err(e) = sender.send(
                                    persistence::ModelEvent::UpdateBlockAgentViewVisibility {
                                        block_id: block_id.to_string(),
                                        agent_view_visibility: agent_view_visibility.into(),
                                    },
                                ) {
                                    log::error!(
                                        "Error sending UpdateBlockAgentViewVisibility event: {e:?}"
                                    );
                                }
                            }
                        }
                    }
                }

                // If we updated the AI context as a result of changing block selection,
                // we don't need to re-sync the selected blocks here.
                if *requires_block_resync {
                    let mut block_indices = Vec::with_capacity(pending_context_block_ids.len());
                    for block_id in pending_context_block_ids.iter() {
                        let Some(block_index) =
                            self.model.lock().block_list().block_index_for_id(block_id)
                        else {
                            continue;
                        };
                        block_indices.push(block_index);
                    }
                    self.change_block_selections_to_match_ai_context(
                        |selected_blocks| selected_blocks.reset_to_block_indices(block_indices),
                        ctx,
                    );
                }

                // If we updated the AI context as a result of changing text selection,
                // we don't need to re-sync the selected text here.
                //
                // Note it's not possible to re-sync non-empty text selections because we have
                // no way of locating text selections from the string alone. In any case, the
                // only current use case for re-syncing selected text is when the pending
                // context is cleared.
                if *requires_text_resync && pending_context_selected_text.is_none() {
                    self.clear_selected_text(ctx);
                }

                // Propagate context model's directory context to the most recent AI block.
                let pwd = context_model.as_ref(ctx).current_pwd();
                let home = context_model.as_ref(ctx).home_directory();
                if let Some(last_block) = self.last_ai_block() {
                    last_block.update(ctx, |block, ctx| {
                        block.update_directory_context(pwd.clone(), home.clone(), ctx);
                    });
                }

                ctx.notify();
            }
            BlocklistAIContextEvent::PendingQueryStateUpdated => {
                self.update_context_blocks_and_exchanges(ctx);

                // When the pending query state is updated (i.e. a conversation is selected or un-selected),
                // update the title to reflect that selected conversation change.
                self.update_pane_configuration(ctx);
                ctx.notify();
            }
        }
    }

    fn remove_pending_cloud_mode_query_if_exchange_has_renderable_user_query(
        &mut self,
        ai_block_model: &AIBlockModelImpl<AIBlock>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.pending_user_query_kind != Some(PendingUserQueryKind::CloudMode) {
            return;
        }

        let initial_conversation_query = ai_block_model
            .conversation(ctx)
            .and_then(|conversation| conversation.initial_user_query());
        let has_renderable_user_query = ai_block_model.inputs_to_render(ctx).iter().any(|input| {
            input
                .display_user_query(initial_conversation_query.as_ref())
                .is_some()
        });
        if has_renderable_user_query {
            self.remove_pending_user_query_block(ctx);
        }
    }
    fn render_owner_for_ai_history_event(
        &self,
        history_model: &BlocklistAIHistoryModel,
        event: &BlocklistAIHistoryEvent,
    ) -> Option<EntityId> {
        match event {
            BlocklistAIHistoryEvent::AppendedExchange {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::UpdatedConversationArtifacts {
                conversation_id, ..
            } => history_model.terminal_view_id_for_conversation(conversation_id),
            BlocklistAIHistoryEvent::ReassignedExchange {
                new_conversation_id,
                ..
            } => history_model.terminal_view_id_for_conversation(new_conversation_id),
            BlocklistAIHistoryEvent::UpdatedConversationMetadata {
                conversation_id, ..
            } => history_model.terminal_view_id_for_conversation(conversation_id),
            BlocklistAIHistoryEvent::StartedNewConversation { .. }
            | BlocklistAIHistoryEvent::CreatedSubtask { .. }
            | BlocklistAIHistoryEvent::UpgradedTask { .. }
            | BlocklistAIHistoryEvent::SetActiveConversation { .. }
            | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
            | BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. }
            | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
            | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
            | BlocklistAIHistoryEvent::SplitConversation { .. }
            | BlocklistAIHistoryEvent::RemoveConversation { .. }
            | BlocklistAIHistoryEvent::DeletedConversation { .. }
            | BlocklistAIHistoryEvent::RestoredConversations { .. }
            | BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. }
            | BlocklistAIHistoryEvent::ConversationOwnershipTransferred { .. }
            | BlocklistAIHistoryEvent::NewConversationRequestComplete { .. }
            | BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
            | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
            | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => None,
        }
    }

    pub(super) fn handle_ai_history_model_event(
        &mut self,
        history_model: ModelHandle<BlocklistAIHistoryModel>,
        event: &BlocklistAIHistoryEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let history_model_ref = history_model.as_ref(ctx);
        let should_handle = match self.render_owner_for_ai_history_event(history_model_ref, event) {
            Some(owner_terminal_view_id) => owner_terminal_view_id == self.view_id,
            None => event
                .terminal_view_id()
                .is_none_or(|terminal_view_id| terminal_view_id == self.view_id),
        };
        if !should_handle {
            return;
        }
        // If the conversation details panel is open and showing an active local
        // AI conversation in this terminal view, refresh its data when status,
        // artifacts, exchanges, or metadata change. Mirrors the WASM transcript
        // panel refresh logic in `Workspace::handle_history_model_event` for
        // APP-3595.
        if self.is_conversation_details_panel_open
            && matches!(
                event,
                BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
                    | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
                    | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
                    | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
                    | BlocklistAIHistoryEvent::AppendedExchange { .. }
                    | BlocklistAIHistoryEvent::SetActiveConversation { .. }
                    | BlocklistAIHistoryEvent::RestoredConversations { .. }
            )
        {
            self.fetch_and_update_conversation_details_panel(ctx);
        }
        if matches!(
            event,
            BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
                | BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. }
        ) {
            self.maybe_insert_tombstone_for_non_running_shared_ambient_task(ctx);
        }
        match event {
            BlocklistAIHistoryEvent::AppendedExchange {
                exchange_id,
                task_id,
                conversation_id,
                is_hidden,
                response_stream_id,
                ..
            } => {
                // Hide telemetry banner forever after first AI input user sends.
                if FeatureFlag::GlobalAIAnalyticsBanner.is_enabled()
                    && !GeneralSettings::as_ref(ctx)
                        .telemetry_banner_dismissed
                        .value()
                {
                    self.hide_telemetry_banner_permanently(ctx);
                }

                // Close any open usage footer(s) when a new AI block is added
                if !self.usage_footer_view_ids.is_empty() {
                    let owner_block_ids: Vec<EntityId> =
                        self.usage_footer_view_ids.keys().copied().collect();
                    for owner_id in &owner_block_ids {
                        if let Some(ai_block_handle) = self.ai_block_handle_by_view_id(*owner_id) {
                            ai_block_handle.update(ctx, |block, ctx| {
                                block.handle_action(
                                    &AIBlockAction::ToggleIsUsageFooterExpanded,
                                    ctx,
                                );
                            });
                        }
                    }
                }

                if self.is_ambient_agent_session(ctx)
                    && self
                        .model
                        .lock()
                        .block_list()
                        .is_executing_oz_environment_startup_commands()
                {
                    self.model
                        .lock()
                        .block_list_mut()
                        .set_is_executing_oz_environment_startup_commands(false);
                }

                // For an oz local-to-cloud handoff, the first `AppendedExchange` is the
                // analogue of `HarnessCommandStarted` for non-oz harnesses: the moment we
                // tear down the queued-prompt block in favor of the live agent UI.
                if self
                    .ambient_agent_view_model
                    .as_ref()
                    .is_some_and(|model| model.as_ref(ctx).is_local_to_cloud_handoff())
                {
                    self.remove_pending_user_query_block(ctx);
                }

                let should_add_ai_block = history_model
                    .as_ref(ctx)
                    .conversation(conversation_id)
                    .and_then(|conversation| conversation.get_task(task_id))
                    .is_some_and(blocklist_filter::should_show_task_in_blocklist);
                if !should_add_ai_block {
                    // Only add AI blocks to the blocklist for root task exchanges (normal Agent Mode)
                    // and advice subagent exchanges (so advice tool calls/results are visible).
                    return;
                }

                let ai_block_model = match AIBlockModelImpl::<AIBlock>::new(
                    *exchange_id,
                    *conversation_id,
                    false,
                    false,
                    ctx,
                ) {
                    Ok(ai_block_model) => ai_block_model,
                    Err(err) => {
                        log::warn!(
                            "Failed to create model for AI block on AppendedExchange. {err}"
                        );
                        return;
                    }
                };
                self.remove_pending_cloud_mode_query_if_exchange_has_renderable_user_query(
                    &ai_block_model,
                    ctx,
                );
                let ai_block = ctx.add_typed_action_view(|ctx| {
                    AIBlock::new(
                        Rc::new(ai_block_model),
                        self.model.clone(),
                        ClientIdentifiers {
                            conversation_id: *conversation_id,
                            client_exchange_id: *exchange_id,
                            response_stream_id: response_stream_id.clone(),
                        },
                        self.ai_controller.clone(),
                        self.get_relevant_files_controller.clone(),
                        self.pwd(),
                        self.shell_launch_data_if_local(ctx),
                        self.ai_action_model.clone(),
                        self.ai_context_model.clone(),
                        self.find_model.clone(),
                        self.active_session.clone(),
                        &self.cli_subagent_controller,
                        &self.model_events_handle,
                        self.agent_view_controller.clone(),
                        self.ambient_agent_view_model.clone(),
                        self.view_handle.clone(),
                        self.view_id,
                        ctx,
                    )
                });

                // Focus the AI block so that the user can cancel,
                // unless the terminal is not even focused which can
                // happen during autonomous execution.
                self.focus_ai_block_if_self_focused(&ai_block, ctx);

                ctx.subscribe_to_view(&ai_block, move |me, block, event, ctx| {
                    me.handle_ai_block_event(
                        block.clone(),
                        false, // is_restored
                        event,
                        ctx,
                    );
                });
                let ai_block_clone = ai_block.clone();
                let is_passive_conversation =
                    ai_block_clone.as_ref(ctx).is_passive_conversation(ctx);
                self.find_model.update(ctx, move |find_model, _ctx| {
                    find_model.register_findable_rich_content_view(ai_block_clone);
                });

                self.insert_rich_content(
                    Some(RichContentType::AIBlock),
                    ai_block.clone(),
                    Some(RichContentMetadata::AIBlock(AIBlockMetadata {
                        exchange_id: *exchange_id,
                        conversation_id: *conversation_id,
                        ai_block_handle: ai_block,
                    })),
                    RichContentInsertionPosition::Append {
                        insert_below_long_running_block: false,
                    },
                    ctx,
                );

                if !is_hidden && !is_passive_conversation {
                    // Clear agent mode query banners and hidden blocks when a new AI block is created.
                    self.clear_prompt_suggestions(ctx);
                    self.drop_hidden_passive_ai_blocks(ctx);
                }

                self.update_context_blocks_and_exchanges(ctx);

                ctx.notify();
            }
            BlocklistAIHistoryEvent::ReassignedExchange {
                exchange_id,
                new_conversation_id,
                ..
            } => {
                if let Some(ai_block_rich_content) =
                    self.rich_content_views.iter_mut().find(|rich_content| {
                        rich_content
                            .ai_block_metadata()
                            .is_some_and(|ai_metadata| ai_metadata.exchange_id == *exchange_id)
                    })
                {
                    if let Some(RichContentMetadata::AIBlock(AIBlockMetadata {
                        ref mut conversation_id,
                        ai_block_handle,
                        ..
                    })) = ai_block_rich_content.metadata_mut()
                    {
                        *conversation_id = *new_conversation_id;
                        let new_model = match AIBlockModelImpl::<AIBlock>::new(
                            *exchange_id,
                            *new_conversation_id,
                            false,
                            false,
                            ctx,
                        ) {
                            Ok(new_model) => new_model,
                            Err(err) => {
                                log::warn!(
                                    "Failed to create model for AI block on ReassignedExchange. {err}"
                                );
                                return;
                            }
                        };

                        ai_block_handle.update(ctx, |block, ctx| {
                            block.reset_conversation_id(
                                *new_conversation_id,
                                Rc::new(new_model),
                                ctx,
                            );
                        });
                    }

                    if FeatureFlag::AgentView.is_enabled() {
                        ai_block_rich_content
                            .update_agent_view_conversation_id(*new_conversation_id);
                        self.model
                            .lock()
                            .block_list_mut()
                            .update_agent_view_conversation_id_for_rich_content(
                                ai_block_rich_content.view_id(),
                                Some(*new_conversation_id),
                            );
                    }
                }
            }
            BlocklistAIHistoryEvent::StartedNewConversation {
                new_conversation_id,
                ..
            } => {
                // If a new conversation has been started, update context block and
                // AI exchange IDs in the `ai_render_context` for the active and inactive
                // conversations.
                let mut ai_render_context = self.ai_render_context.borrow_mut();
                ai_render_context.selected_conversation_id = Some(*new_conversation_id);

                ai_render_context.exchange_ids = Some(HashSet::new());
            }
            BlocklistAIHistoryEvent::UpdatedStreamingExchange {
                exchange_id,
                conversation_id,
                ..
            } => {
                let ai_block_model = match AIBlockModelImpl::<AIBlock>::new(
                    *exchange_id,
                    *conversation_id,
                    false,
                    false,
                    ctx,
                ) {
                    Ok(ai_block_model) => ai_block_model,
                    Err(err) => {
                        log::warn!(
                            "Failed to create model for AI block on UpdatedStreamingExchange. {err}"
                        );
                        self.update_context_blocks_and_exchanges(ctx);
                        return;
                    }
                };
                self.remove_pending_cloud_mode_query_if_exchange_has_renderable_user_query(
                    &ai_block_model,
                    ctx,
                );
                self.update_context_blocks_and_exchanges(ctx);
            }
            BlocklistAIHistoryEvent::SetActiveConversation { .. } => {
                // When the conversation state changes or a new conversation
                // is selected, update the title to reflect that change.
                self.update_pane_configuration(ctx);
            }
            BlocklistAIHistoryEvent::ClearedActiveConversation {
                conversation_id, ..
            } => {
                // When the conversation state changes or a new conversation
                // is selected, update the title to reflect that change.
                self.update_pane_configuration(ctx);

                if FeatureFlag::AgentView.is_enabled() {
                    let rich_content_ids = self
                        .rich_content_views
                        .extract_if(.., |rich_content| {
                            rich_content.agent_view_conversation_id() == Some(*conversation_id)
                        })
                        .map(|rich_content| rich_content.view_id());
                    let mut terminal_model = self.model.lock();
                    for id in rich_content_ids {
                        terminal_model.block_list_mut().remove_rich_content(id);
                    }
                    ctx.notify();
                }
            }
            BlocklistAIHistoryEvent::SplitConversation { .. } => {
                // When the conversation state changes or a new conversation
                // is selected, update the title to reflect that change.
                self.update_pane_configuration(ctx);
            }
            BlocklistAIHistoryEvent::UpdatedConversationStatus {
                conversation_id,
                update,
                ..
            } => {
                // When the conversation state changes or a new conversation
                // is selected, update the title to reflect that change.
                self.update_pane_configuration(ctx);

                // Don't send notifications or insert ambient agent session ended tombstone
                // if we're restoring this conversation on startup.
                if matches!(update, ConversationStatusUpdate::Restored) {
                    return;
                }

                self.maybe_send_agent_mode_desktop_notification(conversation_id, ctx);

                // Show AI credits modal for cloud-mode out-of-credits failures.
                if FeatureFlag::CloudMode.is_enabled()
                    && self.is_ambient_agent_session(ctx)
                    && !self.model.lock().is_shared_ambient_agent_session()
                {
                    if let Some(conversation) =
                        BlocklistAIHistoryModel::as_ref(ctx).conversation(conversation_id)
                    {
                        if matches!(
                            conversation_output_status_from_conversation(conversation),
                            Some(AmbientConversationStatus::Error {
                                error: RenderableAIError::QuotaLimit { .. }
                            })
                        ) {
                            self.show_out_of_credits_modal(ctx);
                        }
                    }
                }

                // For conversation transcript viewers (on WASM) and shared ambient sessions on
                // non-CloudModeSetupV2 paths, insert a conversation-ended tombstone when the
                // conversation completes.
                // We only insert the tombstone once per session (when the conversation finishes).
                // Skip during historical replay to avoid premature tombstone insertion.
                let should_insert_tombstone = if self.conversation_ended_tombstone_view_id.is_none()
                    && !self.model.lock().is_receiving_agent_conversation_replay()
                {
                    #[cfg(target_family = "wasm")]
                    {
                        let model = self.model.lock();
                        // On WASM, keep transcript viewers on the conversation-driven path.
                        // Shared ambient sessions under CloudModeSetupV2 are handled via
                        // AgentConversationsModel task liveness updates instead.
                        model.is_conversation_transcript_viewer()
                            || (!FeatureFlag::CloudModeSetupV2.is_enabled()
                                && model.is_shared_ambient_agent_session())
                    }
                    #[cfg(not(target_family = "wasm"))]
                    {
                        // Show tombstone for shared ambient agent sessions
                        self.model.lock().is_shared_ambient_agent_session()
                            && !FeatureFlag::CloudModeSetupV2.is_enabled()
                    }
                } else {
                    false
                };

                if should_insert_tombstone {
                    if let Some(conversation) =
                        BlocklistAIHistoryModel::as_ref(ctx).conversation(conversation_id)
                    {
                        if !conversation.status().is_in_progress()
                            && conversation_output_status_from_conversation(conversation).is_some()
                        {
                            self.insert_conversation_ended_tombstone_with_cta(None, ctx);
                        }
                    }
                }
            }
            BlocklistAIHistoryEvent::ClearedConversationsInTerminalView {
                active_conversation_id,
                ..
            } => {
                if let Some(active_conversation_id) = active_conversation_id {
                    self.ai_controller.update(ctx, |controller, ctx| {
                        controller.cancel_conversation_progress(
                            *active_conversation_id,
                            CancellationReason::ManuallyCancelled,
                            ctx,
                        );
                    });
                }
                // When the active conversation is invalidated, fall back to the original pane title
                self.pane_configuration.update(ctx, |pane_config, ctx| {
                    pane_config.set_title(self.terminal_title.clone(), ctx);
                });
                self.is_using_conversation_for_pane_header_title = false;
            }
            BlocklistAIHistoryEvent::ConversationOwnershipTransferred {
                conversation_id,
                previous_terminal_view_id,
                ..
            } => {
                // The conversation has moved to another terminal view. We are
                // the previous owner (the per-view filter at the top of this
                // function uses `previous_terminal_view_id`), so drop any
                // rendered blocks tagged to this conversation. Leave the
                // agent-view entry in place so the user can click it to
                // navigate to the current owner pane later.
                if *previous_terminal_view_id != self.view_id {
                    return;
                }
                let view_ids_to_remove = self
                    .rich_content_views
                    .iter()
                    .filter_map(|view| {
                        let is_ai_block_for_conversation = matches!(
                            view.metadata(),
                            Some(RichContentMetadata::AIBlock(metadata))
                                if metadata.conversation_id == *conversation_id
                        );
                        is_ai_block_for_conversation.then_some(view.view_id())
                    })
                    .collect_vec();
                for view_id_to_remove in view_ids_to_remove.into_iter() {
                    self.model
                        .lock()
                        .block_list_mut()
                        .remove_rich_content(view_id_to_remove);
                    self.rich_content_views
                        .retain(|view| view.view_id() != view_id_to_remove);
                }
                self.model
                    .lock()
                    .block_list_mut()
                    .remove_command_blocks_for_conversation(*conversation_id);
            }
            BlocklistAIHistoryEvent::RemoveConversation { .. }
            | BlocklistAIHistoryEvent::DeletedConversation { .. } => {
                // The queue is always for the currently active conversation; agent-view exit
                // already wipes it via `ExitedAgentView`, so no per-conversation cleanup is
                // needed here.
            }
            BlocklistAIHistoryEvent::CreatedSubtask { .. }
            | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. }
            | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
            | BlocklistAIHistoryEvent::RestoredConversations { .. }
            | BlocklistAIHistoryEvent::UpgradedTask { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationMetadata { .. }
            | BlocklistAIHistoryEvent::UpdatedConversationArtifacts { .. }
            | BlocklistAIHistoryEvent::ConversationServerTokenAssigned { .. }
            | BlocklistAIHistoryEvent::NewConversationRequestComplete { .. }
            | BlocklistAIHistoryEvent::OrchestrationConfigUpdated { .. }
            | BlocklistAIHistoryEvent::ConversationUsageMetadataUpdated { .. }
            | BlocklistAIHistoryEvent::LocalSharedSessionEstablished { .. } => {}
        }
        ctx.notify();
    }

    pub(super) fn handle_cli_subagent_controller_event(
        &mut self,
        _: ModelHandle<CLISubagentController>,
        event: &CLISubagentEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CLISubagentEvent::SpawnedSubagent {
                task_id,
                block_id,
                conversation_id,
                initial_requested_command_action_id,
            } => {
                let subagent_view = ctx.add_typed_action_view(|ctx| {
                    CLISubagentView::new(
                        block_id.clone(),
                        self.ai_action_model.clone(),
                        self.cli_subagent_controller.clone(),
                        self.model.clone(),
                        *conversation_id,
                        task_id.clone(),
                        self.pwd(),
                        self.shell_launch_data_if_local(ctx),
                        ctx,
                    )
                });
                self.cli_subagent_views
                    .insert(block_id.clone(), subagent_view.clone());

                ctx.subscribe_to_view(&subagent_view, |me, view, event, ctx| match event {
                    CLISubagentViewEvent::TextSelected => {
                        // Unlike AI blocks, CLI subagent view text selections should not coexist
                        // with block list or alt screen text selections. Clear those first before
                        // calling `clear_selected_text_except`, which handles the side effects
                        // (clipboard sync, context model, etc.) and clears other rich content views.
                        {
                            let mut model = me.model.lock();
                            model.block_list_mut().clear_selection();
                            model.alt_screen_mut().clear_selection();
                        }
                        // Also reset transient terminal-side selection state so stale alt-screen
                        // selection visuals don't persist after switching selection focus to the
                        // CLI subagent view.
                        me.is_selecting = false;
                        me.block_text_selection_start_position = None;
                        me.clear_selected_text_except(Some(view.id()), ctx);
                        ctx.notify();
                    }
                    CLISubagentViewEvent::CopiedEmptyText => {
                        me.copy(ctx);
                    }
                    #[cfg(windows)]
                    CLISubagentViewEvent::WindowsCtrlC => {
                        me.ctrl_c(ctx);
                    }
                });

                if let Some(initial_requested_command_id) = initial_requested_command_action_id {
                    // Remove the AI block for the request/response pair that resulted in spawning
                    // the CLI subagent. You can think of this block as corresponding to the
                    // initial action result input for the long running requested command (the
                    // initial output snapshot) and the output containing the subagent tool call.
                    //
                    // This block doesn't actually have any renderable inputs or outputs (typically,
                    // the output tool call would be rendered like all our other tool calls), but
                    // the CLI subagent tool call is 'special' in that it corresponds to UI rendered
                    // on the _command_ block for the previously requested command.
                    //
                    // We remove this block so the AI block originally containing the requested
                    // command remains immediately above the actual command block in the blocklist,
                    // which enables visual continuity in the requested command's expanded state
                    // (e.g. the expanded requested command header appears right on top of the
                    // running command block; they appear part of the same UI component).
                    if let Some((result_ai_block_id, result_conversation_id, result_exchange_id)) =
                        self.rich_content_views.iter().find_map(|view| {
                            let ai_metadata = view.ai_block_metadata()?;
                            ai_metadata
                                .ai_block_handle
                                .as_ref(ctx)
                                .contains_action_result(initial_requested_command_id, ctx)
                                .then_some((
                                    view.view_id(),
                                    ai_metadata.conversation_id,
                                    ai_metadata.exchange_id,
                                ))
                        })
                    {
                        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                            history_model.set_exchange_hidden_status(
                                self.view_id,
                                result_conversation_id,
                                result_exchange_id,
                                true,
                                ctx,
                            );
                        });
                        self.rich_content_views
                            .retain(|rich_content| rich_content.view_id() != result_ai_block_id);
                        self.model
                            .lock()
                            .block_list_mut()
                            .remove_rich_content(result_ai_block_id);
                    }
                }
            }
            CLISubagentEvent::UpdatedControl {
                agent_has_control, ..
            } => {
                self.redetermine_terminal_focus(ctx);
                self.emit_long_running_command_agent_interaction_state_changed(
                    *agent_has_control,
                    ctx,
                );
            }
            CLISubagentEvent::FinishedSubagent {
                block_id,
                conversation_id,
                ..
            } => {
                self.cli_subagent_views.remove(block_id);

                if FeatureFlag::AgentView.is_enabled() {
                    let Some(conversation_id) = conversation_id else {
                        return;
                    };

                    if self.has_existing_lrc_agent_view_block(*conversation_id)
                        || self
                            .agent_view_controller
                            .as_ref(ctx)
                            .agent_view_state()
                            .is_active()
                    {
                        return;
                    }

                    // In the case that the user has taken control and already exited the agent view,
                    // we insert the corresponding agent view block on command finish instead.
                    self.insert_agent_view_entry_block(
                        AgentViewEntryBlockParams {
                            conversation_id: *conversation_id,
                            is_new: true,
                            is_restored: false,
                            origin: AgentViewEntryOrigin::LongRunningCommand,
                            agent_view_controller: self.agent_view_controller.clone(),
                        },
                        RichContentInsertionPosition::Append {
                            insert_below_long_running_block: true,
                        },
                        ctx,
                    );
                    ctx.notify();
                }
            }
            CLISubagentEvent::ToggledHideResponses => {}
            CLISubagentEvent::UpdatedLastSnapshot => {}
            CLISubagentEvent::ControlHandedBackAfterTransfer => {
                // Notify the shell command executor that control was handed back after transfer.
                self.ai_action_model
                    .as_ref(ctx)
                    .shell_command_executor(ctx)
                    .update(ctx, |executor, _| {
                        executor.notify_control_handed_back();
                    });
            }
        }
    }

    pub(super) fn handle_continue_conversation(
        &mut self,
        conversation_id: &AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        if FeatureFlag::AgentView.is_enabled() {
            self.enter_agent_view_for_conversation(
                None,
                AgentViewEntryOrigin::ContinueConversationButton,
                *conversation_id,
                ctx,
            );
        } else {
            // Set pending query as follow-up in context model
            self.ai_context_model.update(ctx, |context_model, ctx| {
                context_model.set_pending_query_state_for_existing_conversation(
                    *conversation_id,
                    AgentViewEntryOrigin::ContinueConversationButton,
                    ctx,
                );
            });

            // Set input config to AI mode, preserving user's autodetect preference
            self.ai_input_model.update(ctx, |input_model, ctx| {
                input_model.set_input_type(
                    InputType::AI,
                    Some(InputTypeAutoDetectionSource::ContinueConversation),
                    ctx,
                );
            });
        }

        // Send telemetry for follow-up toggle
        send_telemetry_from_ctx!(
            TelemetryEvent::AgentModeContinueConversationButtonClicked {
                conversation_id: *conversation_id,
            },
            ctx
        );

        // Focus the input
        self.redetermine_global_focus(ctx);
    }

    pub(super) fn handle_resume_conversation(
        &mut self,
        conversation_id: &AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        // If `AgentView` is enabled, this button is only rendered when the agent view is already
        // active for the selected conversation, so this call is redundant.
        if !FeatureFlag::AgentView.is_enabled() {
            self.ai_context_model.update(ctx, |context_model, ctx| {
                context_model.set_pending_query_state_for_existing_conversation(
                    *conversation_id,
                    AgentViewEntryOrigin::ResumeConversationButton,
                    ctx,
                )
            });
        }

        self.ai_controller.update(ctx, |controller, ctx| {
            controller.resume_conversation(
                *conversation_id,
                /*can_attempt_resume_on_error*/ true,
                /*is_auto_resume_after_error*/ false,
                vec![],
                ctx,
            );
        });
    }

    /// Handle the opening and closing of the usage footer.
    /// We insert the usage footer as a rich content view into the blocklist
    /// below the block that triggered the toggle event.
    pub(super) fn handle_usage_footer_toggled(
        &mut self,
        source_ai_block_view_id: EntityId,
        conversation_id: AIConversationId,
        is_expanded: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        // Close any existing usage footer for this specific AI block
        if let Some(id) = self.usage_footer_view_ids.remove(&source_ai_block_view_id) {
            let mut model = self.model.lock();
            model.block_list_mut().remove_rich_content(id);
            drop(model);
            self.rich_content_views.retain(|rc| rc.view_id() != id);
        }

        if !is_expanded {
            // If the goal was to close the usage footer block, we've done that above
            ctx.notify();
            return;
        }

        // Get the conversation from the history model
        let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).conversation(&conversation_id)
        else {
            log::error!("Could not find conversation for usage footer");
            return;
        };

        let tool_usage = conversation.tool_usage_metadata();
        let time_to_first_token_ms = conversation.time_to_first_token_for_last_user_query_ms();
        let total_agent_response_time_ms =
            conversation.total_agent_response_time_since_last_user_query_ms();
        let wall_to_wall_response_time_ms =
            conversation.wall_to_wall_response_time_since_last_query();

        let conversation_usage_info = ConversationUsageInfo {
            credits_spent: conversation.credits_spent(),
            credits_spent_for_last_block: conversation.credits_spent_for_last_block(),
            tool_calls: tool_usage.total_tool_calls(),
            models: conversation.token_usage().to_vec(),
            context_window_usage: conversation.context_window_usage(),
            files_changed: tool_usage.apply_file_diff_stats.files_changed,
            lines_added: tool_usage.apply_file_diff_stats.lines_added,
            lines_removed: tool_usage.apply_file_diff_stats.lines_removed,
            commands_executed: tool_usage.run_command_stats.commands_executed,
        };

        let timing_info = TimingInfo {
            time_to_first_token_ms,
            total_agent_response_time_ms,
            wall_to_wall_response_time_ms,
        };

        // View to hold the usage footer. Always route through the
        // rollup-aware constructor so the view subscribes to history
        // events and re-renders when any contributing agent's usage
        // updates. The rollup itself is computed at render time and is
        // self-gating: conversations without descendants short-circuit
        // to today's UI inside `ConversationUsageView::render`, so no
        // feature flag check is needed at the call site.
        //
        // Use `add_typed_action_view` (not `add_view`) so the framework
        // registers `ConversationUsageView::handle_action`. Without this,
        // typed actions like `ToggleDetailsExpanded` / `ShowAllAgentRows`
        // dispatched from the view's own click handlers would be logged
        // as `Dispatched action has no handlers` and silently ignored.
        let usage_view = ctx.add_typed_action_view(|ctx| {
            ConversationUsageView::new_footer_with_rollup(
                conversation_usage_info,
                Some(timing_info),
                MouseStateHandle::default(),
                conversation_id,
                ctx,
            )
        });
        self.usage_footer_view_ids
            .insert(source_ai_block_view_id, usage_view.id());

        let agent_view_conversation_id = self
            .agent_view_controller
            .as_ref(ctx)
            .agent_view_state()
            .active_conversation_id();

        let item = RichContentItem::new(None, usage_view.id(), agent_view_conversation_id, false);

        let mut model = self.model.lock();
        let inserted = model.block_list_mut().insert_rich_content_after_item(
            RemovableBlocklistItem::RichContent(source_ai_block_view_id),
            item,
        );
        drop(model);

        if inserted {
            self.rich_content_views.push(
                RichContent::new(usage_view, agent_view_conversation_id)
                    .with_metadata(RichContentMetadata::UsageFooter),
            );
        } else {
            // Fallback: append usage block to the end of the blocklist
            self.insert_rich_content(
                None,
                usage_view,
                Some(RichContentMetadata::UsageFooter),
                RichContentInsertionPosition::Append {
                    insert_below_long_running_block: true,
                },
                ctx,
            );
        }

        ctx.notify();
    }

    pub(super) fn toggle_usage_footer(&mut self, ctx: &mut ViewContext<Self>) {
        let conversation_id = self
            .agent_view_controller
            .as_ref(ctx)
            .agent_view_state()
            .active_conversation_id();

        let Some(conversation_id) = conversation_id else {
            return;
        };

        let last_ai_block_handle = self
            .rich_content_views
            .iter()
            .rev()
            .find_map(|rich_content| {
                let ai_metadata = rich_content.ai_block_metadata()?;
                (ai_metadata.conversation_id == conversation_id)
                    .then(|| ai_metadata.ai_block_handle.clone())
            });

        if let Some(ai_block_handle) = last_ai_block_handle {
            ai_block_handle.update(ctx, |block, ctx| {
                block.handle_action(&AIBlockAction::ToggleIsUsageFooterExpanded, ctx);
            });
        }
    }

    /// Returns true if the window is wide enough to auto-open side panels.
    pub fn can_auto_open_panel(&self) -> bool {
        self.size_info.pane_width_px().as_f32() > MINIMUM_WIDTH_TO_AUTO_OPEN_PANE
    }

    /// Returns true if conditions are met to auto-open the code review panel:
    /// - Inside a git repository
    /// - Window is wide enough to support the code review panel
    fn can_auto_open_code_review_panel(&self, _ctx: &ViewContext<Self>) -> bool {
        self.current_repo_path.is_some() && self.can_auto_open_panel()
    }

    fn toggle_or_open_code_review_pane(
        &mut self,
        delta_pref: GitDeltaPreference,
        entrypoint: CodeReviewPaneEntrypoint,
        cli_agent: Option<super::CLIAgent>,
        focus_new_pane: bool,
        event_constructor: impl Fn(CodeReviewPanelArg) -> Event,
        ctx: &mut ViewContext<Self>,
    ) {
        let arg = CodeReviewPanelArg {
            repo_path: self.current_repo_path.clone(),
            terminal_view: self.view_handle.clone(),
            entrypoint,
            focus_new_pane,
            cli_agent,
        };

        match delta_pref {
            GitDeltaPreference::Always => {
                ctx.emit(event_constructor(arg));
            }
            GitDeltaPreference::OnlyDirty => {
                // For remote repos, skip the dirty check — there's no local
                // GitRepoStatusModel, so the deferred open would never resolve.
                // The diff chip only appears when the remote shell reports changes,
                // so the user intent is clear.
                if self
                    .current_repo_path
                    .as_ref()
                    .is_some_and(|p| p.is_remote())
                {
                    ctx.emit(event_constructor(arg));
                } else {
                    // Check if repo has uncommitted changes via the per-repo sub-model.
                    #[cfg(feature = "local_fs")]
                    {
                        let is_dirty = self
                            .git_status_metadata(ctx)
                            .map(|m| !m.stats_against_head.has_no_changes());
                        match is_dirty {
                            Some(true) => ctx.emit(event_constructor(arg)),
                            // Metadata not loaded yet — defer until the next
                            // git repo status update delivers it.
                            None => {
                                self.deferred_code_review_open = Some(DeferredCodeReviewOpen {
                                    git_delta_preference: delta_pref,
                                    focus_new_pane,
                                });
                            }
                            Some(false) => {}
                        }
                    }
                }
            }
        }
    }

    pub fn toggle_code_review_pane(
        &mut self,
        delta_pref: GitDeltaPreference,
        entrypoint: CodeReviewPaneEntrypoint,
        cli_agent: Option<super::CLIAgent>,
        focus_new_pane: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.toggle_or_open_code_review_pane(
            delta_pref,
            entrypoint,
            cli_agent,
            focus_new_pane,
            Event::ToggleCodeReviewPane,
            ctx,
        )
    }

    pub fn open_code_review_pane(
        &mut self,
        delta_pref: GitDeltaPreference,
        entrypoint: CodeReviewPaneEntrypoint,
        cli_agent: Option<super::CLIAgent>,
        focus_new_pane: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.toggle_or_open_code_review_pane(
            delta_pref,
            entrypoint,
            cli_agent,
            focus_new_pane,
            Event::OpenCodeReviewPane,
            ctx,
        )
    }

    fn update_context_blocks_and_exchanges(&mut self, ctx: &mut ViewContext<Self>) {
        // If there is new active conversation history, update the context block
        // and AI exchange IDs in the `ai_render_context` for the active conversations.
        let mut ai_render_context = self.ai_render_context.borrow_mut();
        let Some(conversation) = self.ai_context_model.as_ref(ctx).selected_conversation(ctx)
        else {
            // If we're starting a new conversation, there should be no active block or exchange IDs.
            ai_render_context
                .block_ids
                .remove(&AIContextInclusionState::Active);
            ai_render_context.exchange_ids = None;
            ai_render_context.should_highlight_context = false;
            return;
        };
        ai_render_context.should_highlight_context = true;
        let active_conversation_historical_ai_context_block_ids = conversation
            .get_root_task()
            .into_iter()
            .flat_map(|task| {
                task.all_contexts().filter_map(|context| {
                    if let AIAgentContext::Block(block) = context {
                        Some(block.id.clone())
                    } else {
                        None
                    }
                })
            })
            .collect();
        ai_render_context.block_ids.insert(
            AIContextInclusionState::Active,
            active_conversation_historical_ai_context_block_ids,
        );

        let exchange_ids = blocklist_filter::exchanges_for_blocklist(conversation)
            .into_iter()
            .map(|exchange| exchange.id)
            .collect();

        let _ = ai_render_context.exchange_ids.insert(exchange_ids);
    }

    pub(super) fn handle_ai_input_model_event(
        &mut self,
        _ai_input_model: ModelHandle<BlocklistAIInputModel>,
        event: &BlocklistAIInputEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            BlocklistAIInputEvent::InputTypeChanged { config } => {
                self.ai_render_context.borrow_mut().is_ai_input_enabled = config.input_type.is_ai();

                // Force re-render all AIBlocks to ensure that selected text is recolored properly
                self.rerender_rich_content_blocks(ctx);

                // Emit AppStateChanged when the AI input mode changes to trigger pane state saving
                ctx.emit(Event::AppStateChanged);
                ctx.notify();
            }
            BlocklistAIInputEvent::LockChanged { .. } => {}
        }
    }

    pub(super) fn handle_ai_action_model_event(
        &mut self,
        action_model: ModelHandle<BlocklistAIActionModel>,
        event: &BlocklistAIActionEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            BlocklistAIActionEvent::ActionBlockedOnUserConfirmation(_) => {
                let is_agent_in_control = self
                    .model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_in_control();
                if is_agent_in_control {
                    self.redetermine_terminal_focus(ctx);
                }
            }
            BlocklistAIActionEvent::ExecutingAction(..) => {
                self.redetermine_terminal_focus(ctx);
                ctx.notify();
            }
            BlocklistAIActionEvent::FinishedAction { action_id, .. } => {
                // Refresh git line changes when files are potentially updated by an action
                let action_result = action_model
                    .as_ref(ctx)
                    .get_action_result(action_id)
                    .cloned();

                let maybe_modified_files = action_result
                    .as_ref()
                    .map(|result| {
                        matches!(
                            result.result,
                            AIAgentActionResultType::RequestCommandOutput { .. }
                                | AIAgentActionResultType::RequestFileEdits { .. }
                        )
                    })
                    .unwrap_or(false);

                if maybe_modified_files {
                    self.refresh_warp_prompt(ctx);
                    ctx.notify();
                }

                // Auto-open code review pane on first accepted file edits
                if let Some(result) = action_result {
                    if let AIAgentActionResultType::RequestFileEdits(
                        crate::ai::agent::RequestFileEditsResult::Success { .. },
                    ) = &result.result
                    {
                        let history_model = BlocklistAIHistoryModel::handle(ctx);
                        if let Some(conversation_id) = history_model
                            .as_ref(ctx)
                            .conversation_id_for_action(action_id, ctx.view_id())
                        {
                            let already_opened = history_model
                                .as_ref(ctx)
                                .conversation(&conversation_id)
                                .map(|c| c.has_opened_code_review())
                                .unwrap_or(false);

                            let should_auto_open = *GeneralSettings::as_ref(ctx)
                                .auto_open_code_review_pane_on_first_agent_change;

                            if !already_opened
                                && should_auto_open
                                && FeatureFlag::AutoOpenCodeReviewPane.is_enabled()
                                && self.can_auto_open_code_review_panel(ctx)
                                // we shouldn't auto open if this was triggered from a passive code review diff
                                && !BlocklistAIHistoryModel::as_ref(ctx)
                                .is_entirely_passive_conversation(&conversation_id)
                            {
                                self.open_code_review_pane(
                                    GitDeltaPreference::Always,
                                    CodeReviewPaneEntrypoint::ForceOpened,
                                    None,  // cli_agent
                                    false, /* focus_new_pane */
                                    ctx,
                                );
                            }
                        }
                    }
                }
            }
            BlocklistAIActionEvent::InitProject(_) => {
                self.on_next_conversation_finished(|me, _reason, ctx| {
                    if let Some(path) = me.pwd() {
                        CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
                            manager.index_directory(PathBuf::from(path), ctx);
                        });
                        me.ai_controller.update(ctx, |controller, ctx| {
                            controller.send_slash_command_request(
                                SlashCommandRequest::InitProjectRules,
                                ctx,
                            );
                        });
                    }
                });
            }
            BlocklistAIActionEvent::ToggleCodeReview(_) => {
                self.toggle_code_review_pane(
                    GitDeltaPreference::Always,
                    CodeReviewPaneEntrypoint::InvokedByAgent,
                    None,  // cli_agent
                    false, /* focus_new_pane */
                    ctx,
                );
            }
            BlocklistAIActionEvent::InsertCodeReviewComments {
                action_id: _,
                repo_path,
                comments,
                base_branch,
            } => {
                if !FeatureFlag::PRCommentsV2.is_enabled() {
                    self.handle_insert_code_review_comments_event(
                        repo_path,
                        comments,
                        base_branch.as_deref(),
                        ctx,
                    );
                }
            }
            BlocklistAIActionEvent::QueuedAction(_) => {}
        }
    }

    fn handle_insert_code_review_comments_event(
        &mut self,
        repo_path: &Path,
        comments: &[InsertReviewComment],
        base_branch: Option<&str>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::PRCommentsSlashCommand.is_enabled() {
            return;
        }
        let pending_comments = convert_insert_review_comments(comments);

        if pending_comments.is_empty() {
            log::warn!("No valid comments to insert");
            return;
        }

        // Determine DiffMode from the base branch.
        if self.current_repo_path.is_none() {
            log::error!("Cannot insert PR comments: not in a git repository");
            return;
        }

        let diff_mode = self.diff_mode_for_branch(base_branch, ctx);

        let open_code_review = if FeatureFlag::PRCommentsV2.is_enabled() {
            None
        } else {
            Some(CodeReviewPanelArg {
                repo_path: Some(LocalOrRemotePath::Local(repo_path.to_path_buf())),
                terminal_view: self.view_handle.clone(),
                entrypoint: CodeReviewPaneEntrypoint::InvokedByAgent,
                focus_new_pane: false,
                cli_agent: None,
            })
        };

        ctx.emit(Event::InsertCodeReviewComments {
            repo_path: LocalOrRemotePath::Local(repo_path.to_path_buf()),
            comments: pending_comments,
            diff_mode,
            open_code_review,
        });
    }

    /// Gets the DiffMode for the given branch name by fetching the main branch name
    /// for this session and comparing it to the given branch name.
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub(super) fn diff_mode_for_branch(
        &self,
        base_branch: Option<&str>,
        ctx: &mut ViewContext<Self>,
    ) -> DiffMode {
        match base_branch {
            Some(branch) => {
                #[cfg(feature = "local_fs")]
                let main_branch = self
                    .git_status_metadata(ctx)
                    .map(|m| m.main_branch_name.clone())
                    .and_then(|mb| mb.strip_prefix("origin/").map(String::from));
                #[cfg(not(feature = "local_fs"))]
                let main_branch: Option<String> = None;
                DiffMode::from_branch(branch, main_branch.as_deref())
            }
            None => DiffMode::MainBranch,
        }
    }

    pub(super) fn handle_shell_command_executor_event(
        &mut self,
        _: ModelHandle<ShellCommandExecutor>,
        event: &ShellCommandExecutorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ShellCommandExecutorEvent::ExecuteCommand { command, action_id } => {
                let Some(session_id) = self.active_block_session_id() else {
                    return;
                };

                let history_model = BlocklistAIHistoryModel::as_ref(ctx);
                let Some(conversation) = history_model
                    .conversation_id_for_action(action_id, ctx.view_id())
                    .and_then(|id| history_model.conversation(&id))
                else {
                    safe_error!(
                        safe: ("No conversation ID found for command with ID: {:?}", action_id),
                        full: (
                            "No conversation ID found for requested command: ID: {:?}, command: \
                            {command}",
                            action_id
                        )
                    );
                    return;
                };
                let associated_workflow = self
                    .active_ai_block(ctx)
                    .and_then(|ai_block| {
                        ai_block
                            .as_ref(ctx)
                            .requested_command_copied_from_doc(action_id, ctx)
                    })
                    .and_then(|citation| {
                        if let AIAgentCitation::WarpDriveObject { uid } = citation {
                            CloudModel::as_ref(ctx).get_workflow_by_uid(&uid)
                        } else {
                            None
                        }
                    });

                let shell_family = self.sessions.read(ctx, |sessions, _| {
                    sessions
                        .get(session_id)
                        .map(|session| session.shell_family())
                });
                // We special-case PowerShell multi-line commands b/c PowerShell does not support
                // bracketed paste escape codes. Without those, multi-line commands have each line
                // appear in a separate block. However, Agent Mode assumes the command will be in a
                // single block. This is technically an issue with other shells without bracketed
                // paste support, e.g. Bash 3.2, so this workaround is an incomplete solution. We
                // wrap the PowerShell snipped in an immediately invoked script block.
                let command = match shell_family {
                    Some(ShellFamily::PowerShell) => {
                        let command = command.trim();
                        if command.contains('\n') {
                            format!(". {{ {command} }}")
                        } else {
                            command.to_string()
                        }
                    }
                    _ => command.clone(),
                };

                let workflow_telem_metadata = associated_workflow.map(|workflow| {
                    let workflow_data = &workflow.model().data;
                    WorkflowTelemetryMetadata {
                        workflow_source: workflow.space(ctx).into(),
                        workflow_categories: workflow_data.tags().cloned(),
                        workflow_selection_source: WorkflowSelectionSource::AgentMode,
                        workflow_id: workflow.sync_id().into_server().map(Into::into),
                        workflow_space: Some(workflow.space(ctx).into()),
                        enum_ids: workflow_data.get_server_enum_ids(),
                    }
                });

                let agent_metadata =
                    AgentInteractionMetadata::new_hidden(action_id.clone(), conversation.id());

                // We use the basic AI source when this is a non-shared
                // command originating from the agent.
                let mut source = CommandExecutionSource::AI {
                    metadata: agent_metadata.clone(),
                };

                let model = self.model.lock();
                if model.shared_session_status().is_sharer() {
                    if let Some(participant_id) = self
                        .shared_session_presence_manager()
                        .map(|m| m.as_ref(ctx).id())
                    {
                        // If this is a shared session, we use the SharedSession source
                        // with ai metadata for the terminal command included.
                        source = CommandExecutionSource::SharedSession {
                            participant_id: participant_id.clone(),
                            block_id: model.block_list().active_block_id().clone(),
                            ai_metadata: Some(agent_metadata.clone()),
                        }
                    }
                }
                let block_id = model.active_block_id().clone();
                drop(model);

                ctx.emit(Event::ExecuteCommand(ExecuteCommandEvent {
                    command,
                    session_id,
                    source,
                    should_add_command_to_history: true,
                    workflow_id: associated_workflow.map(|workflow| workflow.sync_id()),
                    workflow_command: associated_workflow
                        .and_then(|workflow| workflow.model().data.command())
                        .map(str::to_string),
                }));

                if let Some(active_ai_block) = self.active_ai_block(ctx) {
                    active_ai_block.update(ctx, |block, ctx| {
                        block.on_requested_command_execution_started(action_id.clone(), ctx)
                    });
                }

                // If the command turns out to be long-running, lock the input in agent mode.
                ctx.spawn(
                    // Command execution is triggered by a subscriber to the event above, so
                    // give some buffer to actually determine if its long running.
                    Timer::after(Duration::from_millis(LONG_RUNNING_COMMAND_DURATION_MS * 2)),
                    move |me, _, ctx| {
                        if me
                            .model
                            .lock()
                            .block_list()
                            .block_with_id(&block_id)
                            .is_some_and(|block| block.is_active_and_long_running())
                        {
                            me.input.update(ctx, |input, ctx| {
                                input.set_input_mode_agent(false, ctx);
                            });
                        }
                    },
                );

                if let Some(metadata) = workflow_telem_metadata {
                    send_telemetry_from_ctx!(TelemetryEvent::WorkflowExecuted(metadata), ctx);
                }
                ctx.notify();
            }
            ShellCommandExecutorEvent::WriteToPty { input, mode } => {
                self.write_agent_bytes_to_pty(input.to_vec(), mode, ctx);
            }
            ShellCommandExecutorEvent::CancelExecution => {
                // We need to manually invoke ctrl-c to terminate the running command because the
                // user's ctrl-c was directed to the AIBlock instead of the command's shell block.
                self.ctrl_c(ctx);
            }
            ShellCommandExecutorEvent::TransferControlToUser { reason, .. } => {
                // Transfer control of the long-running command to the user.
                self.cli_subagent_controller.update(ctx, |controller, ctx| {
                    controller.switch_control_to_user(
                        UserTakeOverReason::TransferFromAgent {
                            reason: reason.clone(),
                        },
                        ctx,
                    );
                });
            }
        }
    }

    pub(super) fn handle_start_agent_executor_event(
        &mut self,
        _executor: ModelHandle<StartAgentExecutor>,
        event: &StartAgentExecutorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            StartAgentExecutorEvent::CreateAgent(request) => {
                ctx.emit(Event::StartAgentConversation(request.clone()));
            }
        }
    }

    pub(super) fn get_ai_notification_summary(
        &self,
        conversation: &AIConversation,
        app: &AppContext,
    ) -> Option<AIBlockNotificationSummary> {
        let title = conversation.title()?.to_string();

        if conversation.status().is_blocked() {
            let reason = self
                .ai_action_model
                .as_ref(app)
                .get_pending_action(app)
                .map(|action| match &action.action {
                    AIAgentActionType::RequestCommandOutput { command, .. } => {
                        format!("Oz needs your permission to run `{command}`")
                    }
                    AIAgentActionType::ReadFiles(..) => {
                        "Oz needs your permission to read files".to_string()
                    }
                    AIAgentActionType::SearchCodebase(..) => {
                        "Oz needs your permission to search your codebase".to_string()
                    }
                    AIAgentActionType::RequestFileEdits { .. } => {
                        "Oz needs your permission to edit a file".to_string()
                    }
                    AIAgentActionType::WriteToLongRunningShellCommand { .. } => {
                        "Oz needs your permission to interact with a running shell command"
                            .to_string()
                    }
                    _ => "Oz needs your confirmation to continue".to_string(),
                })
                .unwrap_or("Oz needs your confirmation to continue".to_string());
            return Some(AIBlockNotificationSummary {
                success: false,
                title,
                description: reason,
            });
        } else if conversation.status().is_in_progress() {
            return None;
        }

        let last_exchange = conversation.root_task_exchanges().last()?;
        match &last_exchange.output_status {
            AIAgentOutputStatus::Finished {
                finished_output, ..
            } => {
                match finished_output {
                    FinishedAIAgentOutput::Success { output, .. } => {
                        // Get last line of output for summary
                        let last_line = output
                            .get()
                            .text_from_agent_output()
                            .last()
                            .and_then(|text| {
                                text.sections.iter().find_map(|section| match section {
                                    AIAgentTextSection::PlainText { text } => {
                                        Some(text.text().to_string())
                                    }
                                    _ => None,
                                })
                            })
                            .unwrap_or_default();

                        Some(AIBlockNotificationSummary {
                            success: true,
                            title: title.clone(),
                            description: last_line,
                        })
                    }
                    FinishedAIAgentOutput::Error {
                        error: RenderableAIError::Other { error_message, .. },
                        ..
                    } => Some(AIBlockNotificationSummary {
                        success: false,
                        title: title.clone(),
                        description: error_message.clone(),
                    }),
                    _ => Some(AIBlockNotificationSummary {
                        success: false,
                        title,
                        description: "An unknown error occurred".to_string(),
                    }),
                }
            }
            _ => None,
        }
    }
}
