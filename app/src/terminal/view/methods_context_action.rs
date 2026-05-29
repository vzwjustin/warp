use super::*;

impl TerminalView {
    pub(super) fn context_menu_action(
        &mut self,
        action: &ContextMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        use ContextMenuAction::*;

        // TODO: handle sharing session with > 1 block selected
        let source = SharedSessionActionSource::BlocklistContextMenu {
            block_index: self.selected_blocks.tail(),
        };
        match action {
            InsertSelectedText => self.context_menu_insert_selected_text(ctx),
            CopySelectedText => self.context_menu_copy_selected_text(ctx),
            CopyUrl { url_content } => self.context_menu_copy_url(url_content, ctx),
            CopyBlocks => self.context_menu_copy_blocks(ctx),
            CopyBlockCommands => self.context_menu_copy_block_commands(ctx),
            CopyBlockOutputs => self.context_menu_copy_block_outputs(ctx),
            OpenShareBlockModal { block_index } => {
                self.context_menu_open_share_block_modal(*block_index, ctx)
            }
            FindWithinBlock => self.find_within_block(ctx),
            ScrollToBottomOfBlock => self.scroll_to_bottom_of_bottommost_selected_block(ctx),
            ScrollToTopOfBlock => self.scroll_to_top_of_topmost_selected_block(ctx),
            ToggleBookmark => self.bookmark_selected_block(ctx),
            CopyPrompt { position, part } => self.copy_prompt(position, part, ctx),
            CopyRprompt => self.copy_rprompt(ctx),
            EditPrompt => self.edit_prompt(ctx),
            EditAgentToolbar => {
                if FeatureFlag::AgentToolbarEditor.is_enabled() {
                    ctx.emit(Event::OpenAgentToolbarEditor);
                }
            }
            EditCLIAgentToolbar => {
                if FeatureFlag::AgentToolbarEditor.is_enabled() {
                    ctx.emit(Event::OpenCLIAgentToolbarEditor);
                }
            }
            AskAI(ask_source) => {
                if FeatureFlag::AgentMode.is_enabled() {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::AgentModeClickedEntrypoint {
                            entrypoint: AgentModeEntrypoint::ContextMenu {
                                selection_type: if matches!(
                                    ask_source,
                                    AskAISource::SelectedBlockOrText
                                        | AskAISource::SelectedTerminalText
                                        | AskAISource::SelectedInputText
                                ) {
                                    telemetry::AgentModeEntrypointSelectionType::Text
                                } else {
                                    // The `AskAI` action for the context menu is only triggered
                                    // with selected text or selected block(s).
                                    telemetry::AgentModeEntrypointSelectionType::Block
                                }
                            },
                        },
                        ctx
                    );
                }

                self.ask_ai(ask_source, ctx);
            }
            OpenWorkflowModal => self.open_workflow_modal(ctx),
            OpenShareSessionModal => self.open_share_session_modal(source, ctx),
            StopSharing => self.stop_sharing_session(source, ctx),
            CopyBlockFilteredOutputs => self.context_menu_copy_filtered_block_outputs(ctx),
            CopyAIDebuggingLink {
                conversation_token,
                request_id,
            } => {
                let url = match request_id {
                    Some(request_id) => {
                        format!("{}?request={}", conversation_token.debug_link(), request_id)
                    }
                    None => conversation_token.debug_link(),
                };
                ctx.clipboard().write(ClipboardContent::plain_text(url));
            }
            CopyAIBlockQuery { ai_block_view_id } => {
                for rich_content in self.rich_content_views.iter() {
                    if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                        if ai_metadata.ai_block_handle.id() == *ai_block_view_id {
                            ai_metadata.ai_block_handle.update(ctx, |block, ctx| {
                                block.handle_action(&AIBlockAction::CopyQuery, ctx);
                            });
                            break;
                        }
                    }
                }
            }
            CopyAIBlockOutput { ai_block_view_id } => {
                // Copy only the current AI block's output
                for rich_content in self.rich_content_views.iter() {
                    if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                        if ai_metadata.ai_block_handle.id() == *ai_block_view_id {
                            ai_metadata.ai_block_handle.update(ctx, |block, ctx| {
                                block.handle_action(&AIBlockAction::CopyOutput, ctx);
                            });
                            break;
                        }
                    }
                }
            }
            CopyAIBlock { ai_block_view_id } => {
                // Copy current AI block's prompt and output
                for rich_content in self.rich_content_views.iter() {
                    if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                        if ai_metadata.ai_block_handle.id() == *ai_block_view_id {
                            ai_metadata.ai_block_handle.update(ctx, |block, ctx| {
                                block.handle_action(&AIBlockAction::Copy, ctx);
                            });
                            break;
                        }
                    }
                }
            }
            CopyAIBlockConversation { ai_block_view_id } => {
                let conversation_id = self.rich_content_views.iter().find_map(|rich_content| {
                    let ai_metadata = rich_content.ai_block_metadata()?;
                    (ai_metadata.ai_block_handle.id() == *ai_block_view_id)
                        .then_some(ai_metadata.conversation_id)
                });
                if let Some(conversation_id) = conversation_id {
                    self.copy_conversation_text(conversation_id, ctx);
                }
            }
            CopyExternalDebuggingId {
                request_id,
                conversation_id,
            } => {
                let debug_info = if let Some(request_id) = request_id {
                    format!(
                        "{{\"request_id\":\"{}\",\"conversation_id\":\"{}\"}}",
                        request_id,
                        conversation_id.as_str()
                    )
                } else {
                    format!("{{\"conversation_id\":\"{}\"}}", conversation_id.as_str())
                };
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(debug_info));
            }
            CopyConversationId { conversation_id } => {
                ctx.clipboard().write(ClipboardContent::plain_text(
                    conversation_id.as_str().to_string(),
                ));
            }
            CopyServerRequestId { request_id } => {
                ctx.clipboard().write(ClipboardContent::plain_text(
                    request_id.as_str().to_string(),
                ));
            }
            CopyConversationShareLink { conversation_id } => {
                if let Some(link) = ShareableObject::AIConversation(*conversation_id).link(ctx) {
                    ctx.clipboard().write(ClipboardContent::plain_text(link));
                }
            }
            CopyConversationText { conversation_id } => {
                self.copy_conversation_text(*conversation_id, ctx);
            }
            ForkAIConversation { conversation_id } => {
                self.fork_ai_conversation(*conversation_id, None, ctx);
            }
            OpenConversationShareDialog { conversation_id } => {
                // Set the shareable object and open the sharing dialog via the pane header
                let shareable_object = ShareableObject::AIConversation(*conversation_id);
                self.pane_configuration.update(ctx, |pane_config, ctx| {
                    pane_config.set_shareable_object(Some(shareable_object), ctx);
                    pane_config.toggle_sharing_dialog(SharingDialogSource::AIBlockContextMenu, ctx);
                });
            }
            CopyAgentCommand { ai_block_view_id } => {
                for rich_content in self.rich_content_views.iter() {
                    if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                        if ai_metadata.ai_block_handle.id() == *ai_block_view_id {
                            ai_metadata.ai_block_handle.update(ctx, |block, ctx| {
                                block.handle_action(&AIBlockAction::CopyCommand, ctx);
                            });
                            break;
                        }
                    }
                }
            }
            CopyAgentGitBranch { ai_block_view_id } => {
                for rich_content in self.rich_content_views.iter() {
                    if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                        if ai_metadata.ai_block_handle.id() == *ai_block_view_id {
                            let ai_block = ai_metadata.ai_block_handle.as_ref(ctx);
                            let model = self.model.lock();
                            let git_branch =
                                ai_block
                                    .requested_commands_iter()
                                    .find_map(|(action_id, _)| {
                                        model
                                            .block_list()
                                            .block_for_ai_action_id(action_id)
                                            .and_then(|block| block.git_branch().cloned())
                                    });

                            if let Some(branch) = git_branch {
                                ctx.clipboard().write(ClipboardContent::plain_text(branch));
                            }
                            break;
                        }
                    }
                }
            }
            ForkAIConversationFromBlock {
                ai_block_view_id: _,
                exchange_id,
                conversation_id,
            } => {
                self.fork_ai_conversation(
                    *conversation_id,
                    Some(ForkFromExchange {
                        exchange_id: *exchange_id,
                        fork_from_exact_exchange: false,
                    }),
                    ctx,
                );
            }
            ForkAIConversationFromExactExchange {
                ai_block_view_id: _,
                exchange_id,
                conversation_id,
            } => {
                self.fork_ai_conversation(
                    *conversation_id,
                    Some(ForkFromExchange {
                        exchange_id: *exchange_id,
                        fork_from_exact_exchange: true,
                    }),
                    ctx,
                );
            }
            SavePromptAsAgentModeWorkflow { ai_block_view_id } => {
                for rich_content in self.rich_content_views.iter() {
                    if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                        if ai_metadata.ai_block_handle.id() == *ai_block_view_id {
                            let prompt_text = ai_metadata
                                .ai_block_handle
                                .as_ref(ctx)
                                .get_preceding_user_query(ctx);
                            ctx.emit(Event::OpenAddPromptPane {
                                initial_content: Some(prompt_text),
                            });
                            break;
                        }
                    }
                }
            }
        }
    }

    pub(super) fn show_rewind_confirmation_dialog(
        &mut self,
        ai_block_view_id: EntityId,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
        entrypoint: AgentModeRewindEntrypoint,
        ctx: &mut ViewContext<Self>,
    ) {
        send_telemetry_from_ctx!(
            TelemetryEvent::AgentModeRewindDialogOpened { entrypoint },
            ctx
        );
        ctx.dispatch_typed_action(&WorkspaceAction::ShowRewindConfirmationDialog {
            ai_block_view_id,
            exchange_id,
            conversation_id,
        });
    }

    pub(super) fn rewind_ai_conversation(
        &mut self,
        ai_block_view_id: EntityId,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        // First, cancel any in-progress conversation
        self.ai_controller.update(ctx, |controller, ctx| {
            controller.cancel_conversation_progress(
                conversation_id,
                CancellationReason::Reverted,
                ctx,
            );
        });

        // If the active block is a running command from this conversation, stop it and
        // set the take-over reason to Stop to prevent automatic conversation resume.
        let should_stop_running_command = {
            let mut model = self.model.lock();
            let active_block = model.block_list_mut().active_block_mut();

            let is_from_this_conversation = active_block.is_executing()
                && active_block.ai_conversation_id() == Some(conversation_id);

            if is_from_this_conversation {
                active_block.set_user_control_with_stop_reason();
            }

            is_from_this_conversation
        };

        // Note: CTRL-C isn't guaranteed to stop everything, such as a Python REPL.
        if should_stop_running_command {
            self.user_write_ctrl_c_to_pty(ctx);
        }

        // Iterate from end backwards, reverting all diffs in each AIBlock from this conversation until the block the user clicked on (inclusive)
        let mut num_blocks_reverted = 0;
        for rich_content in self.rich_content_views.iter().rev() {
            if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                // Only revert blocks from the same conversation
                if ai_metadata.conversation_id == conversation_id {
                    ai_metadata.ai_block_handle.update(ctx, |block, ctx| {
                        block.revert_all_diffs(ctx);
                    });
                    num_blocks_reverted += 1;
                    if ai_metadata.ai_block_handle.id() == ai_block_view_id {
                        break;
                    }
                }
            }
        }

        // Save a backup of the conversation before truncating, so users can restore it later.
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
            if let Some(conversation) = history_model.conversation(&conversation_id).cloned() {
                if let Err(e) = history_model.fork_conversation(
                    &conversation,
                    PRE_REWIND_PREFIX,
                    false, /* preserve_task_ids */
                    None,
                    ctx,
                ) {
                    log::warn!("Failed to save pre-rewind backup of conversation {conversation_id}: {e}");
                }
            } else {
                log::warn!("Failed to save pre-rewind backup: conversation {conversation_id} not found in memory");
            }
        });

        // Truncate the conversation history
        let removed_exchange_ids =
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                history_model.truncate_conversation_from_exchange(conversation_id, exchange_id, ctx)
            });

        // Truncate the blocklist UI
        match removed_exchange_ids {
            Ok(removed_ids) => {
                self.remove_ai_blocks_for_exchanges(&conversation_id, &removed_ids, ctx);
            }
            Err(e) => {
                log::warn!("Failed to truncate conversation: {e}");
            }
        }

        // Clear stale action results that reference truncated tool calls.
        self.ai_controller.update(ctx, |controller, ctx| {
            controller.clear_finished_action_results(conversation_id, ctx);
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::AgentModeRewindExecuted {
                num_blocks_reverted
            },
            ctx
        );
    }

    pub(super) fn handle_input_context_menu_action(
        &mut self,
        action: &InputContextMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        use InputContextMenuAction::*;

        match action {
            CutSelectedText => self.cut_selected_text_from_input(ctx),
            CopySelectedText => self.copy_selected_text_from_input(ctx),
            SelectAll => self.select_all_text_from_input(ctx),
            Paste => self.paste_in_input(ctx),
            ShowCommandSearch => self.command_search_from_input(ctx),
            AskWarpAI => self.ask_ai(&AskAISource::SelectedInputText, ctx),
            ShowAICommandSearch => self.ai_command_search_from_input(ctx),
            SaveAsWorkflow => self.save_as_workflow_from_input(ctx),
            ToggleInputHintText => self.toggle_input_hint_text(ctx),
        }
        self.close_context_menu(ctx, false);
    }

    pub(super) fn selected_block_accessibility_content(
        &mut self,
        index: BlockIndex,
    ) -> Option<AccessibilityContent> {
        let model = self.model.lock();
        model.block_list().block_at(index).map(|block| {
            let status = if block.has_failed() {
                format!("failed, status code {}", block.exit_code().value())
            } else if block.is_background() {
                "background".to_string()
            } else if block.is_done() {
                "succeeded".to_string()
            } else {
                "in progress".to_string()
            };
            AccessibilityContent::new(
                format!("Block {index}: {}, {}.\n", block.command_to_string(), status),
                // TODO (a11y) Keybindings should be taken from the actual user's
                // configuration
                "Press cmd-C to read and copy both command and output, and cmd-option-shift-C to read and copy output only. Press cmd-B to bookmark the block: you could navigate between bookmarked blocks quickly using option-up and option-down.",
                WarpA11yRole::TextRole,
            )
        })
    }

    pub(super) fn notifications_error_banner_action(
        &mut self,
        action: NotificationsErrorBannerAction,
        ctx: &mut ViewContext<Self>,
    ) {
        use NotificationsErrorBannerAction::*;

        match action {
            Troubleshoot => {
                ctx.open_url(NOTIFICATIONS_TROUBLESHOOT_URL);
            }
            Close => self.close_notification_error_banner(ctx),
            SetPermissions => {
                ctx.request_desktop_notification_permissions(move |view, outcome, ctx| {
                    // If the request was accepted, we can close the banner. Otherwise, keep it open, indicating the problem
                    // has not been resolved.
                    if matches!(outcome, RequestPermissionsOutcome::Accepted) {
                        view.close_notification_error_banner(ctx);
                    }
                });
            }
        }

        send_telemetry_from_ctx!(TelemetryEvent::NotificationsErrorBannerAction(action), ctx);
    }

    fn close_notification_error_banner(&mut self, ctx: &mut ViewContext<Self>) {
        if let NotificationsErrorBannerType::Open { state, .. } = &self
            .inline_banners_state
            .notifications_error_banner
            .banner_type
        {
            self.model
                .lock()
                .block_list_mut()
                .remove_inline_banner(state.banner_id);
        }
        self.inline_banners_state
            .notifications_error_banner
            .banner_type = NotificationsErrorBannerType::Closed;
        ctx.notify();
    }

    pub(super) fn notifications_discovery_banner_action(
        &mut self,
        action: NotificationsDiscoveryBannerAction,
        ctx: &mut ViewContext<Self>,
    ) {
        use NotificationsDiscoveryBannerAction::*;

        match action {
            LearnMore => {
                ctx.open_url(NOTIFICATIONS_LEARN_MORE_URL);
            }
            Troubleshoot => {
                ctx.open_url(NOTIFICATIONS_TROUBLESHOOT_URL);
            }
            TurnOn(trigger) => {
                let current_settings = SessionSettings::as_ref(ctx).notifications.value().clone();
                let new_settings = NotificationsSettings {
                    mode: NotificationsMode::Enabled,
                    ..current_settings
                };
                SessionSettings::handle(ctx).update(ctx, |session_settings, ctx| {
                    if let Err(e) = session_settings.notifications.set_value(new_settings, ctx) {
                        log::error!("Error persisting notifications setting: {e}");
                    }
                });

                // On Linux, immediately mark the request permission status as accepted since there's no concept of
                // requesting desktop notification permissions.
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                {
                    if let NotificationsDiscoveryBanner::Open {
                        request_outcome, ..
                    } = &mut self.inline_banners_state.notifications_discovery_banner
                    {
                        *request_outcome = Some(RequestPermissionsOutcome::Accepted);
                    }
                }

                ctx.request_desktop_notification_permissions(move |view, outcome, ctx| {
                    if let NotificationsDiscoveryBanner::Open {
                        request_outcome, ..
                    } = &mut view.inline_banners_state.notifications_discovery_banner
                    {
                        *request_outcome = Some(outcome.clone());
                    }
                    // Log to sentry if unknown error
                    if let RequestPermissionsOutcome::OtherError { error_message } = &outcome {
                        log::error!(
                            "Unknown error when requesting notification permissions. error_msg: {error_message}"
                        );
                    }

                    send_telemetry_from_ctx!(
                        TelemetryEvent::NotificationsRequestPermissionsOutcome { outcome },
                        ctx
                    );
                    ctx.notify();
                });
                send_telemetry_from_ctx!(
                    TelemetryEvent::NotificationPermissionsRequested {
                        source: NotificationsTurnedOnSource::Banner,
                        trigger: Some(trigger),
                    },
                    ctx
                );
                ctx.notify();
            }
            Configure => {
                ctx.emit(Event::OpenSettings(SettingsSection::Features));
            }
            Close => {
                // Update settings to mark notifications as dismissed to prevent banner from showing again
                let current_settings = SessionSettings::as_ref(ctx).notifications.value().clone();
                let new_settings = NotificationsSettings {
                    mode: NotificationsMode::Dismissed,
                    ..current_settings
                };
                SessionSettings::handle(ctx).update(ctx, |session_settings, ctx| {
                    if let Err(e) = session_settings.notifications.set_value(new_settings, ctx) {
                        log::error!("Error persisting notifications setting: {e}");
                    }
                });

                if let NotificationsDiscoveryBanner::Open { state, .. } =
                    &self.inline_banners_state.notifications_discovery_banner
                {
                    self.model
                        .lock()
                        .block_list_mut()
                        .remove_inline_banner(state.banner_id);
                }
                self.inline_banners_state.notifications_discovery_banner =
                    NotificationsDiscoveryBanner::Closed;
                ctx.notify();
            }
        }

        send_telemetry_from_ctx!(
            TelemetryEvent::NotificationsDiscoveryBannerAction(action),
            ctx
        );
    }

    pub(super) fn ssh_banner_action(&self, action: SSHBannerAction, ctx: &mut ViewContext<Self>) {
        use SSHBannerAction::*;

        match action {
            LearnMore => {
                ctx.open_url("https://docs.warp.dev/terminal/warpify/ssh-legacy#implementation");
            }
            Settings => {
                if FeatureFlag::SSHTmuxWrapper.is_enabled() {
                    ctx.emit(Event::OpenSettings(SettingsSection::Warpify));
                } else {
                    ctx.emit(Event::OpenSettings(SettingsSection::Features));
                }
            }
        }
    }

    // Invokes the on_next_frame_drawn API to time from the provided block started at to the moment
    // the frame is drawn.
    // It doesn't matter when this method is called, as long as it's before the next frame is drawn.
    pub(super) fn install_block_latency_telemetry_callback(
        &mut self,
        block_latency_data: BlockLatencyData,
        ctx: &mut ViewContext<Self>,
    ) {
        let session_info = self
            .active_block_session_id()
            .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id))
            .map(|session| {
                let shell_name = session.shell().shell_type().name();
                (session.is_legacy_ssh_session(), shell_name)
            });

        if let Some((is_ssh, shell)) = session_info {
            let auth_state = self.auth_state.clone();
            let executor = ctx.background_executor().clone();
            ctx.on_next_frame_drawn(move || {
                let block_event = TelemetryEvent::BaselineCommandLatency(BlockLatencyInfo {
                    command: block_latency_data.command,
                    shell,
                    is_ssh,
                    // The execution time is from the time the block started (i.e. user hit
                    // enter) to when the first frame after the block completed is finished
                    // drawing.
                    execution_ms: block_latency_data.started_at.elapsed().as_millis() as u64,
                });
                send_telemetry_on_executor!(auth_state, block_event, executor);
            })
        } else {
            log::warn!("Could not log block latency telemetry since session info was none");
        }
    }

    /// Toggles the block filter on the last selected block, or the last non-hidden
    /// block if none are selected.
    ///
    /// When a filter is toggled off, it is set as inactive but the query remains
    /// saved on the block. It can be reactivated by toggling on. If there is no
    /// inactive query, toggling on a filter will simply open the filter editor.
    pub(super) fn toggle_block_filter_on_selected_or_last_block(
        &mut self,
        source: ToggleBlockFilterSource,
        ctx: &mut ViewContext<Self>,
    ) {
        let model = self.model.lock();
        let Some(selected_or_last_block_index) = self
            .selected_blocks
            .tail()
            .or_else(|| model.block_list().last_non_hidden_block_by_index())
        else {
            log::info!("No block found to toggle block filter on");
            return;
        };

        let Some(block_filter_query) = model
            .block_list()
            .block_at(selected_or_last_block_index)
            .map(|block| block.current_filter().cloned())
        else {
            log::warn!("No block found at given block index when toggling filter");
            return;
        };
        drop(model);

        if let Some(block_filter_query) = block_filter_query {
            let new_block_filter_query = BlockFilterQuery {
                is_active: !block_filter_query.is_active,
                ..block_filter_query
            };

            send_telemetry_from_ctx!(
                TelemetryEvent::ToggleBlockFilterQuery {
                    enabled: new_block_filter_query.is_active,
                    source
                },
                ctx
            );

            self.update_block_filter_for_block(
                selected_or_last_block_index,
                &new_block_filter_query,
                ctx,
            );
            if new_block_filter_query.is_active {
                self.open_block_filter_editor(
                    selected_or_last_block_index,
                    OpenedFromClick::No,
                    ctx,
                );
            } else {
                self.close_block_filter_editor(ctx);
                self.redetermine_global_focus(ctx);
            }
        } else {
            self.open_block_filter_editor(selected_or_last_block_index, OpenedFromClick::No, ctx);
        }
    }

    /// Replace the terminal input buffer with the given command that is meant to open a subshell.
    /// Set a flag that we should automatically bootstrap AKA "warpify" the subshell when we
    /// receive the [`AfterBlockStarted`] event.
    pub fn insert_subshell_command_and_bootstrap_if_supported(
        &mut self,
        command: &str,
        shell_type: Option<ShellType>,
        ctx: &mut ViewContext<Self>,
    ) {
        // If the shell type is not supported, it will be None.
        self.pending_auto_bootstrap_shell_type = shell_type;

        self.input.update(ctx, |input, ctx| {
            input.replace_buffer_content(command, ctx);
        });
    }

    fn reset_focus_after_rich_block(&mut self, ctx: &mut ViewContext<Self>) {
        self.redetermine_terminal_focus(ctx);
        self.input.update(ctx, |input, ctx| {
            input.editor().update(ctx, |editor, ctx| {
                editor.clear_autosuggestion(ctx);
            });
        });
    }

    pub fn cancel_env_var_block(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(block) = self.active_env_var_collection_block(ctx) {
            block.update(ctx, |view, ctx| {
                view.cancel(ctx);
            });
        }
    }

    fn add_env_var_block_to_blocklist(
        &mut self,
        collection_title: String,
        command: String,
        session_id: SessionId,
        cloud_object_type_and_id: CloudObjectTypeAndId,
        ctx: &mut ViewContext<Self>,
    ) {
        let block_id = Uuid::new_v4().to_string();
        let env_var_collection_block = ctx.add_typed_action_view(|ctx| {
            EnvVarCollectionBlock::new(block_id.clone(), collection_title, command, ctx)
        });
        env_var_collection_block.update(ctx, |block, ctx| block.focus(ctx));

        ctx.subscribe_to_view(&env_var_collection_block, move |me, block, event, ctx| {
            let event = event.clone();
            match event {
                EnvVarCollectionBlockEvent::RanCommand(command) => {
                    ctx.emit(Event::ExecuteCommand(ExecuteCommandEvent {
                        command,
                        session_id,
                        workflow_id: None,
                        workflow_command: None,
                        should_add_command_to_history: false,
                        source: CommandExecutionSource::EnvVarCollection {
                            metadata: BlocklistEnvVarMetadata {
                                block_id: block_id.clone(),
                                should_hide_block: true,
                            },
                        },
                    }));

                    UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                        update_manager.record_object_action(
                            cloud_object_type_and_id,
                            ObjectActionType::Execute,
                            None,
                            ctx,
                        )
                    });
                    me.reset_focus_after_rich_block(ctx);
                }
                EnvVarCollectionBlockEvent::Cancelled => {
                    // Send the escape code corresponding to ctrl-c, indicating the running command
                    // should be terminated. Note that this will not revert already-run `export`s.
                    me.keydown_on_terminal("\u{0003}", ctx);
                    me.reset_focus_after_rich_block(ctx);
                }
                EnvVarCollectionBlockEvent::ToggledExpanded(block_id) => {
                    me.model
                        .lock()
                        .block_list_mut()
                        .toggle_visibility_of_block_for_env_var(&block_id);
                    me.redetermine_global_focus(ctx);
                    ctx.notify();
                }
                EnvVarCollectionBlockEvent::TextSelected => {
                    me.clear_selected_text_except(Some(block.id()), ctx);
                }
            }

            ctx.notify();
        });

        self.insert_rich_content(
            None,
            env_var_collection_block.clone(),
            Some(RichContentMetadata::EnvVarCollectionBlock {
                env_var_collection_block_handle: env_var_collection_block,
            }),
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: false,
            },
            ctx,
        );
    }

    fn display_non_local_environment_variable_error(
        &self,
        window_id: WindowId,
        ctx: &mut ViewContext<Self>,
    ) {
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            toast_stack.add_ephemeral_toast(
                DismissibleToast::error(
                    "Can not invoke environment variable subshell in a non-local session"
                        .to_owned(),
                ),
                window_id,
                ctx,
            );
        });
    }

    #[allow(unused_variables)]
    pub(super) fn get_shell_starter_local(
        &self,
        ctx: &mut ViewContext<Self>,
    ) -> Option<(String, ShellType)> {
        #[cfg(feature = "local_tty")]
        {
            // TODO(CORE-2300): This appears to be used for invoking env vars.
            // Before we close out CORE-2300, we should evaluate if we need to add
            // shell info here.
            let shell_starter = get_shell_starter(None, &self.auth_state, ctx)?;
            let shell_path = match &shell_starter {
                ShellStarter::Direct(direct_shell_starter)
                | ShellStarter::MSYS2(direct_shell_starter) => direct_shell_starter
                    .shell_path()
                    .to_string_lossy()
                    .to_string(),
                ShellStarter::DockerSandbox(docker_shell_starter) => docker_shell_starter
                    .direct
                    .shell_path()
                    .to_string_lossy()
                    .to_string(),
                ShellStarter::Wsl(wsl_shell_starter) => wsl_shell_starter.shell_path(),
            };
            Some((shell_path, shell_starter.shell_type()))
        }

        #[cfg(not(feature = "local_tty"))]
        None
    }

    pub fn invoke_environment_variables(
        &mut self,
        cloud_env_var_collection: CloudEnvVarCollection,
        in_subshell: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let session_id = self.active_block_session_id();

        if !in_subshell {
            let Some(shell_type) = self.active_session_shell_type(ctx) else {
                return;
            };
            self.invoke_env_vars_in_current_session(
                cloud_env_var_collection.clone(),
                shell_type,
                session_id,
                ctx,
            );
        } else {
            let window_id = ctx.window_id();
            let shell_session_info =
                if self.active_session_is_local(ctx).unwrap_or(false) || !in_subshell {
                    if let Some(shell_info) = self.get_shell_starter_local(ctx) {
                        shell_info
                    } else {
                        // TODO(PR): This can fail for reasons besides being "non-local". We can also
                        // not find a fallback shell.
                        self.display_non_local_environment_variable_error(window_id, ctx);
                        return;
                    }
                } else {
                    self.display_non_local_environment_variable_error(window_id, ctx);
                    return;
                };

            self.invoke_env_vars_in_subshell(
                cloud_env_var_collection,
                shell_session_info,
                window_id,
                ctx,
            );
        }
    }

    fn invoke_env_vars_in_current_session(
        &mut self,
        cloud_env_var_collection: CloudEnvVarCollection,
        shell_type: ShellType,
        session_id: Option<SessionId>,
        ctx: &mut ViewContext<Self>,
    ) {
        let env_var_collection = cloud_env_var_collection.model().string_model.clone();
        if let Some(session_id) = session_id {
            self.add_env_var_block_to_blocklist(
                env_var_collection
                    .title
                    .clone()
                    .unwrap_or("Untitled".to_owned()),
                env_var_collection
                    .vars
                    .iter()
                    .map(|var| var.get_initialization_string(shell_type))
                    .collect_vec()
                    .join(" "),
                session_id,
                cloud_env_var_collection.cloud_object_type_and_id(),
                ctx,
            );
        } else {
            self.pending_env_var_collection = Some(cloud_env_var_collection)
        }
    }

    fn set_and_execute_subshell_command(
        &mut self,
        shell_command: &str,
        shell_type: ShellType,
        ctx: &mut ViewContext<Self>,
    ) {
        // Attempt to auto warpify the subshell when bootstrapped
        self.pending_auto_bootstrap_shell_type = Some(shell_type);

        self.input.update(ctx, |input, ctx| {
            input.set_pending_command(shell_command, ctx);
            input.execute_pending_command(ctx);
        });
    }

    fn invoke_env_vars_in_subshell(
        &mut self,
        cloud_env_var_collection: CloudEnvVarCollection,
        shell_session_info: (String, ShellType),
        window_id: WindowId,
        ctx: &mut ViewContext<Self>,
    ) {
        let env_var_collection = cloud_env_var_collection.model().string_model.clone();

        let (shell_path_string, shell_type) = shell_session_info;
        if shell_type == ShellType::PowerShell {
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                let toast =
                    DismissibleToast::error("PowerShell subshells not supported".to_owned());
                toast_stack.add_ephemeral_toast(toast, window_id, ctx);
            });
            return;
        }

        // Set the env vars before executing a subshell command so that it will be loaded on
        // subshell start
        self.env_vars = env_var_collection.vars;
        self.model.lock().set_env_var_collection_name(Some(
            env_var_collection.title.unwrap_or("Untitled".to_owned()),
        ));
        self.set_and_execute_subshell_command(&shell_path_string, shell_type, ctx);

        // Ok to update the execution record here because we auto-execute when in subshell
        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
            update_manager.record_object_action(
                cloud_env_var_collection.cloud_object_type_and_id(),
                ObjectActionType::Execute,
                None,
                ctx,
            )
        });
    }

    /// Handles when a user clicks on a block in the list of blocks attached to an AI block.
    pub(super) fn scroll_to_and_maybe_select_block(
        &mut self,
        block_index: BlockIndex,
        ctx: &mut ViewContext<Self>,
    ) {
        // Selecting the block makes it clear which the user is looking at. We shouldn't select the
        // block if they're in AI mode because that would affect their pending query's context block
        // selection.
        if !self.ai_input_model.as_ref(ctx).is_ai_input_enabled() {
            self.reset_selection_to_single_block(block_index, ctx);
        }

        self.scroll_to(block_index, ctx);
    }

    pub(crate) fn view_id(&self) -> EntityId {
        self.view_id
    }

    pub(super) fn cursor_position_id(&self) -> String {
        self.cursor_position_id.clone()
    }

    pub(super) fn drag_and_drop_files(&mut self, paths: &[String], ctx: &mut ViewContext<Self>) {
        self.is_file_drop_target = false;
        if paths.is_empty() {
            return;
        }

        // Focus this pane when files are dropped on it.
        self.redetermine_global_focus(ctx);

        // Check if we're in a long-running command
        let is_in_long_running_command = self
            .model
            .lock()
            .block_list()
            .active_block()
            .is_active_and_long_running();

        let image_filepaths = get_image_filepaths_from_paths(paths);

        // CLI-agent paste path: when a CLI agent (e.g. Claude Code) is the
        // foreground long-running process and the user is interacting with its
        // TUI directly (rich input closed), hand image drops to the agent the
        // same way Cmd+V does at `TerminalView::paste` — write each image to
        // the system clipboard and send the agent's paste keystroke to the
        // PTY. Without this branch the path string would be shell-escaped and
        // typed into the agent's prompt. When the rich input is open we leave
        // the existing chip-attach flow alone, since that's where the user
        // explicitly asked the drop to land.
        if !image_filepaths.is_empty()
            && image_filepaths.len() == paths.len()
            && is_in_long_running_command
            && self.has_active_cli_agent_session(ctx)
            && !CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.view_id)
        {
            self.paste_dropped_images_to_cli_agent(image_filepaths, ctx);
            return;
        }

        if !is_in_long_running_command {
            // Check for image file paths to be auto-attached
            let num_images = image_filepaths.len();

            // If we have image file paths, try to process them for attachment
            if num_images > 0 {
                let num_attached = self.input.update(ctx, |input, ctx| {
                    input.handle_pasted_or_dragdropped_image_filepaths(image_filepaths, ctx)
                });

                // If dropped only image file paths, we are done
                if num_attached == paths.len() {
                    return; // Return early, don't insert file paths
                }
            }
        }

        let Some(session) = self
            .active_block_session_id()
            .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id))
        else {
            return;
        };

        let sshed = self.model.lock().is_warpified_ssh() || session.is_legacy_ssh_session();
        if sshed && !paths.is_empty() && FeatureFlag::SshDragAndDrop.is_enabled() {
            self.initiate_ssh_file_upload(paths, ctx);
        } else {
            // For long-running commands in MSYS2/Git Bash on Windows, skip
            // conversion and shell escaping. Executables in git bash
            // aren't git bash _specific_, they still expect paths in
            // the native windows format.
            let is_msys2_long_running = cfg!(windows)
                && !session.is_wsl()
                && session.shell_family() == ShellFamily::Posix
                && is_in_long_running_command;
            if is_msys2_long_running {
                let input = warpui::clipboard_utils::escaped_paths_str(paths, None);
                self.typed_characters_on_terminal(&input, ctx);
                return;
            }

            // For WSL sessions on Windows, convert paths to /mnt/<drive>/... format
            // so the WSL session can read the file at the correct path.
            let paths_converted;
            let paths = if session.is_wsl() {
                paths_converted = paths
                    .iter()
                    .map(|p| warp_util::path::convert_windows_path_to_wsl(p))
                    .collect::<Vec<_>>();
                paths_converted.as_slice()
            } else {
                paths
            };

            let input =
                warpui::clipboard_utils::escaped_paths_str(paths, Some(self.shell_family(ctx)));
            self.typed_characters_on_terminal(&input, ctx);
        }
    }

    pub fn initiate_ssh_file_upload(&self, paths: &[String], ctx: &mut ViewContext<Self>) {
        let remote_pwd = self.pwd();
        if let Some(ssh_connection_info) = self.ssh_session_info(ctx) {
            let Some(ref ssh_host) = ssh_connection_info.host else {
                return;
            };
            self.ssh_file_upload.update(ctx, |file_upload, ctx| {
                file_upload.start_file_upload(
                    ssh_host,
                    paths,
                    &remote_pwd,
                    &ssh_connection_info,
                    ctx,
                )
            });
        }
    }

    pub fn propagate_password_request(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(Event::FileUploadPasswordPending)
    }

    pub fn propagate_upload_finished_event(
        &mut self,
        exit_code: ExitCode,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(Event::FileUploadFinished(exit_code))
    }

    fn ssh_session_info(&self, ctx: &ViewContext<Self>) -> Option<InteractiveSshCommand> {
        let session = self
            .active_block_session_id()
            .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id))?;
        session
            .as_ref()
            .subshell_info()
            .as_ref()
            .and_then(|info| info.ssh_connection_info.clone())
    }

    pub(super) fn warpify_ssh_session(&mut self, ctx: &mut ViewContext<Self>) {
        self.warpify_state.set_shell_detection_in_progress();
        self.begin_ssh_warpify_timeout(SSH_WARPIFY_TIMEOUT_DURATION, ctx);
        self.clear_line_editor_and_write_to_pty(
            convert_script_to_one_line(&begin_warpify_ssh_session_command(ctx)).into_bytes(),
            ctx,
        );
    }

    pub(super) fn continue_warpify_ssh_session(
        &mut self,
        uname: &str,
        shell_type: ShellType,
        ctx: &mut ViewContext<Self>,
    ) {
        self.warpify_state.set_shell_type(&shell_type);
        self.model.lock().set_pending_warp_initiated_control_mode();
        if let Some(script) = warpify_ssh_session_command(uname, shell_type, ctx) {
            self.clear_line_editor_and_write_to_pty_with_mac_workaround_hack(
                convert_script_to_one_line(&script).into_bytes(),
                ctx,
            );
        } else {
            self.add_ssh_error_block(
                WarpificationUnavailableReason::UnsupportedShell {
                    shell_name: shell_type.name().to_string(),
                },
                ctx,
            );
        }
    }

    pub(super) fn install_tmux_and_warpify(
        &mut self,
        ctx: &mut ViewContext<Self>,
        install_method: &TmuxInstallMethod,
    ) {
        let install_with_root_method = install_method.should_use_package_manager;
        let install_script = &install_method.script;
        self.model
            .lock()
            .set_pending_warp_initiated_control_mode_with_install_tmux(install_with_root_method);
        self.clear_line_editor_and_write_to_pty(
            convert_script_to_one_line(install_script).into_bytes(),
            ctx,
        );
    }

    fn begin_ssh_warpify_timeout(&mut self, duration: Duration, ctx: &mut ViewContext<Self>) {
        let timeout_id = self.warpify_state.replace_timeout_id();
        let active_block_id = self.model.lock().block_list().active_block_id().clone();
        let system_details = self
            .warpify_state
            .ssh_block_state()
            .and_then(|s| s.get_system_details(ctx))
            .to_owned();
        self.warpify_state.add_ssh_warpify_timeout_handle(ctx.spawn(
            async move {
                Timer::after(duration).await;
                (timeout_id, active_block_id, system_details)
            },
            |terminal_view, (timeout_id, active_block_id, system_details), ctx| {
                let is_shell_detection =
                    terminal_view.warpify_state.is_shell_detection_in_progress();
                if timeout_id == terminal_view.warpify_state.timeout_id()
                    && terminal_view.model.lock().block_list().active_block_id() == &active_block_id
                {
                    terminal_view.add_ssh_error_block(
                        WarpificationUnavailableReason::Timeout {
                            is_tmux_install: false,
                            is_shell_detection,
                            system_details,
                        },
                        ctx,
                    );
                }
            },
        ));
    }

    pub(super) fn handle_detected_end_of_ssh_login(
        &mut self,
        check_type: &SshLoginStatus,
        ctx: &mut ViewContext<TerminalView>,
    ) {
        match check_type {
            SshLoginStatus::RecheckBeforeWarpifying => {
                // After we receive a line of output from ssh that is NOT prompting for user input (unlike "Enter passphrase: "),
                // we wait and repeat the check after a small delay in case the state returned to something that's user-input bound.
                // For example, say the output that kicked off this event was "Permission denied, please try again." and
                // ssh will subsequently re-prompt for user input. We want to avoid assuming that ssh authentication is completed until
                // we confirm twice that user input is not currently being requested.
                //
                // Note: 100ms is an estimate, not backed by any particular technical happenings.
                let active_block_id = self.model.lock().block_list().active_block_id().clone();
                ctx.spawn(
                    async {
                        warpui::r#async::Timer::after(Duration::from_secs(3)).await;
                        active_block_id
                    },
                    move |terminal_view, active_block_id, _| {
                        let mut model = terminal_view.model.lock();
                        if model.block_list().active_block_id() == &active_block_id {
                            model.check_for_end_of_ssh_login(true);
                        }
                    },
                );
            }
            SshLoginStatus::ReadyToWarpify => {
                // After the confirmation check, we are confident enough to auto-warpify or offer warpification.
                let Some(command) = &self.warpify_state.get_pending_ssh_command() else {
                    return;
                };
                let ssh_host = &self.warpify_state.get_pending_ssh_host();

                let shell_family = self.shell_family(ctx);
                let warpify_settings = WarpifySettings::as_ref(ctx);

                let ssh_interactive_session_event = evaluate_warpify_ssh_host(
                    command,
                    ssh_host.as_deref(),
                    shell_family,
                    warpify_settings,
                );

                if let SshInteractiveSessionDetected::ShouldPromptWarpification {
                    ref host,
                    ref command,
                } = ssh_interactive_session_event
                {
                    if FeatureFlag::WarpifyFooter.is_enabled() {
                        self.show_warpify_footer(
                            WarpificationMode::ssh(command.clone(), host.to_owned()),
                            ctx,
                        );
                    } else {
                        self.add_ssh_warpify_prompt(command, host.to_owned(), ctx)
                    }
                }

                send_telemetry_from_ctx!(
                    TelemetryEvent::SshInteractiveSessionDetected(ssh_interactive_session_event),
                    ctx
                );
            }
        }
    }

    /// Parses the shell launch data and sets the necessary fields so a shell
    /// indicator is rendered in the tab bar and pane header. Does nothing on
    /// non-Windows platforms.
    pub fn on_active_shell_launch_data_updated(
        &mut self,
        shell_launch_data: Option<ShellLaunchData>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !cfg!(windows) {
            return;
        }

        let shell_indicator_type = shell_launch_data
            .as_ref()
            .and_then(|data| ShellIndicatorType::try_from(data).ok());
        self.shell_indicator_type = shell_indicator_type;
        self.shell_detail = shell_launch_data.map(|launch_data| launch_data.shell_detail());

        // Notify pane header to re-render with updated shell indicator.
        self.pane_configuration.update(ctx, |config, ctx| {
            config.notify_header_content_changed(ctx);
        });
    }

    pub fn shell_indicator_type(&self) -> Option<ShellIndicatorType> {
        self.shell_indicator_type
    }

    /// Shows the warpify footer for a detected subshell/SSH command.
    pub(super) fn show_warpify_footer(
        &mut self,
        mode: WarpificationMode,
        ctx: &mut ViewContext<Self>,
    ) {
        let model = self.model.lock();

        // Shared session viewers can't initiate warpification currently.
        // Don't show the warpify footer when an agent is monitoring the command either.
        if model.shared_session_status().is_viewer()
            || model.block_list().active_block().is_agent_monitoring()
        {
            return;
        }
        drop(model);

        let is_ssh = mode.is_ssh();
        self.use_agent_footer.update(ctx, |footer, ctx| {
            footer.set_warpify_mode(mode, ctx);
        });
        self.maybe_show_use_agent_footer_in_blocklist(ctx);

        send_telemetry_from_ctx!(TelemetryEvent::WarpifyFooterShown { is_ssh }, ctx);
    }

    pub(super) fn show_initialization_block(&mut self) {
        self.model
            .lock()
            .block_list_mut()
            .set_show_bootstrap_block(true);
    }

    pub(super) fn generate_codebase_index(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(active_session_path) = self.active_session_path_if_local(ctx) else {
            return;
        };

        CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.build_and_sync_codebase_index(
                BuildSource::FromPath(active_session_path.as_path()),
                ctx,
            );
        });
    }

    pub(super) fn write_codebase_index(&self, _ctx: &mut ViewContext<Self>) {
        #[cfg(feature = "local_fs")]
        {
            let Some(working_directory_str) = self.pwd() else {
                log::error!("No working directory found for terminal session");
                return;
            };

            let working_directory = PathBuf::from(working_directory_str);
            CodebaseIndexManager::handle(_ctx).update(_ctx, |index_manager, ctx| {
                index_manager.write_snapshot(working_directory.as_path(), ctx);
            });
        }
    }

    /// Starts all enabled LSP servers for the current working directory.

    pub(super) fn toggle_file_tree(
        &mut self,
        cli_agent: Option<crate::server::telemetry::CLIAgentType>,
        ctx: &mut ViewContext<Self>,
    ) {
        use crate::server::telemetry::{FileTreeSource, TelemetryEvent};

        self.toggle_left_panel_file_tree(false, ctx);
        send_telemetry_from_ctx!(
            TelemetryEvent::FileTreeToggled {
                source: FileTreeSource::LeftPanelToolbelt,
                is_code_mode_v2: true,
                cli_agent,
            },
            ctx
        );
    }
}

impl Entity for TerminalView {
    type Event = Event;
}

impl TypedActionView for TerminalView {
    type Action = TerminalAction;

    fn action_accessibility_contents(
        &mut self,
        action: &TerminalAction,
        ctx: &mut ViewContext<Self>,
    ) -> ActionAccessibilityContent {
        use ActionAccessibilityContent::*;
        use TerminalAction::*;

        match action {
            BlockHover(_)
            | BlockSnackbarHover { .. }
            | BlockNearSnackbarHover { .. }
            | MaybeLinkHover { .. } => Empty,
            BlockTextSelect(_) => {
                let semantic_selection = SemanticSelection::as_ref(ctx);
                let model = self.model.lock();
                model
                    .selection_to_string(semantic_selection, self.is_inverted_blocklist(ctx), ctx)
                    .map_or(Empty, |selected| {
                        Custom(AccessibilityContent::new_without_help(
                            selected,
                            WarpA11yRole::TextRole,
                        ))
                    })
            }
            BlockSelect { .. }
            | SelectPriorBlock
            | SelectNextBlock
            | SelectBookmarkUp
            | SelectBookmarkDown
            | Up
            | Down
            | JumpToBookmark(_)
            | ScrollToTopOfBlock { topmost_block: _ } => {
                if let Some(content) = self
                    .selected_blocks
                    .tail()
                    .and_then(|index| self.selected_block_accessibility_content(index))
                {
                    Custom(content)
                } else {
                    Empty
                }
            }
            BookmarkBlock(_) | BookmarkSelectedBlock => {
                Custom(AccessibilityContent::new_without_help(
                    "Toggle Bookmark block",
                    WarpA11yRole::TextRole,
                ))
            }
            ExpandBlockSelectionAbove | ExpandBlockSelectionBelow => {
                if let Some(mut content) = self
                    .selected_blocks
                    .tail()
                    .and_then(|index| self.selected_block_accessibility_content(index))
                {
                    let num_selected_text =
                        format!("Selected {} blocks.", self.num_non_hidden_selected_blocks());
                    content.value = format!("{}\n{}", num_selected_text, content.value);
                    Custom(content)
                } else {
                    Empty
                }
            }
            SelectAllBlocks => Custom(AccessibilityContent::new_without_help(
                format!(
                    "Selected all {} blocks.",
                    self.num_non_hidden_selected_blocks()
                ),
                WarpA11yRole::TextRole,
            )),
            ScrollToBottomOfSelectedBlocks => Custom(AccessibilityContent::new_without_help(
                "Scrolled to bottom of selected block".to_string(),
                WarpA11yRole::TextRole,
            )),
            ScrollToTopOfSelectedBlocks => Custom(AccessibilityContent::new_without_help(
                "Scrolled to top of selected block".to_string(),
                WarpA11yRole::TextRole,
            )),
            ScrollToBottomOfOverhangingBlock(_) => Custom(AccessibilityContent::new_without_help(
                "Scrolled to bottom of bottommost visible block".to_string(),
                WarpA11yRole::TextRole,
            )),
            CopyOutputs => {
                let mut outputs = vec![];
                self.with_non_hidden_selected_blocks(
                    |block| {
                        outputs.push(format!(
                            "Block {}.\nOutput: {}",
                            block.index(),
                            block.output_to_string()
                        ));
                    },
                    ctx,
                );
                let text = format!(
                    "Copied {} block outputs.\n{}",
                    outputs.len(),
                    outputs.join("\n")
                );
                Custom(AccessibilityContent::new_without_help(
                    text,
                    WarpA11yRole::TextRole,
                ))
            }
            Copy => {
                let mut blocks = vec![];
                self.with_non_hidden_selected_blocks(
                    |block| {
                        blocks.push(format!(
                            "Block {}: {}. Output: {}",
                            block.index(),
                            block.command_to_string(),
                            block.output_to_string()
                        ));
                    },
                    ctx,
                );
                let text = format!("Copied {} blocks.\n{}", blocks.len(), blocks.join("\n"));
                Custom(AccessibilityContent::new_without_help(
                    text,
                    WarpA11yRole::TextRole,
                ))
            }
            FocusInputAndClearSelection => {
                Custom(AccessibilityContent::new(
                    INPUT_A11Y_LABEL,
                    // TODO (a11y) use bindings from user settings
                    INPUT_A11Y_HELPER,
                    WarpA11yRole::TextareaRole,
                ))
            }
            KeyDown(key) => {
                let label = if key.eq("\x1b") {
                    INPUT_A11Y_LABEL
                } else {
                    key
                };
                Custom(AccessibilityContent::new_without_help(
                    label,
                    WarpA11yRole::TextareaRole,
                ))
            }
            OpenBlockFilterEditor(block_index) => Custom(AccessibilityContent::new_without_help(
                format!("Open block filter editor for block {block_index}"),
                WarpA11yRole::TextRole,
            )),
            ShowInitializationBlock => Custom(AccessibilityContent::new_without_help(
                "Showed initialization block",
                WarpA11yRole::TextareaRole,
            )),
            ShowWarpifySettings => Custom(AccessibilityContent::new_without_help(
                "Opened Warpify Settings",
                WarpA11yRole::ButtonRole,
            )),
            OpenFilesPalette { .. } => Custom(AccessibilityContent::new_without_help(
                "Opened file search palette",
                WarpA11yRole::ButtonRole,
            )),
            InsertCommandCorrection { .. }
            | BlockListContextMenu(_)
            | CloseContextMenu
            | Paste
            | MiddleClickOnGrid { .. }
            | MiddleClickOnInput
            | CopyCommands
            | MaybeHoverSecret { .. }
            | CopyGitBranch
            | OpenShareModal
            | ReinputCommands
            | ReinputCommandsWithSudo
            | ClearBuffer
            | Focus
            | ShowFindBar
            | PageUp
            | PageDown
            | Home
            | End
            | KeyboardSelectText(_)
            | ContextMenu(_)
            | SplitRight(_)
            | SplitLeft(_)
            | SplitDown(_)
            | SplitUp(_)
            | OpenGridLink(_)
            | OpenRichContentLink(_)
            | ToggleGridSecret { .. }
            | ToggleRichContentSecret { .. }
            | CopyGridSecret(_)
            | CopyRichContentSecret(_)
            | ShowInFileExplorer(_)
            | OpenFileInWarp(_)
            | CtrlD
            | CtrlC
            | ClearSelectionsWhenShellMode
            | Close
            | TypedCharacters(_)
            | UserInputSequence(_)
            | ControlSequence(_)
            | TriggerSubshellBootstrap
            | ShowSubshellBanner(_)
            | DismissWarpifyBanner(_)
            | OpenBlockListContextMenu
            | AliasExpansionBanner(_)
            | VimModeBanner(_)
            | InsertMostRecentCommandCorrection
            | StopSharingCurrentSession { .. }
            | RequestSharedSessionRole(_)
            | OnboardingFlow(_)
            | ImportSettings
            | DragAndDropFiles(_)
            | WarpifySSHSession
            | ShowWarpifySshBanner(_, _)
            | NotifySshErrorBlock(_)
            | ToggleBlockFilterOnSelectedOrLastBlock(_)
            | SetMarkedText { .. }
            | ResumeConversation
            | ForkConversationFromLastKnownGoodState
            | ToggleAIDocumentPane
            | ClearMarkedText
            | StartLspServer => ActionAccessibilityContent::from_debug(),
            #[cfg(feature = "local_fs")]
            OpenCodeInWarp { .. } => ActionAccessibilityContent::from_debug(),
            OpenInWarpBanner(action) => self.open_in_warp_banner_accessibility_content(*action),
            OpenAIBlockAttachedBlocksMenu { .. } => Custom(AccessibilityContent::new_without_help(
                "Open list of blocks attached as context to this AI query.".to_owned(),
                WarpA11yRole::PopoverRole,
            )),
            OpenAIBlockOverflowMenu { .. } => Custom(AccessibilityContent::new_without_help(
                "Open overflow menu with copy options for this AI block.".to_owned(),
                WarpA11yRole::PopoverRole,
            )),
            RewindAIConversation { .. } => Custom(AccessibilityContent::new_without_help(
                "Show confirmation dialog to rewind to before this point in the AI conversation."
                    .to_owned(),
                WarpA11yRole::ButtonRole,
            )),
            ExecuteRewindAIConversation { .. } => Custom(AccessibilityContent::new_without_help(
                "Execute rewind to before this point in the AI conversation.".to_owned(),
                WarpA11yRole::ButtonRole,
            )),
            SelectAIAttachedBlock(_) => Custom(AccessibilityContent::new_without_help(
                "Click on a block attached as context to this AI query.".to_owned(),
                WarpA11yRole::ButtonRole,
            )),
            PickRepoToOpen => Custom(AccessibilityContent::new_without_help(
                "Use file picker to select a git repository".to_owned(),
                WarpA11yRole::PopoverRole,
            )),
            #[cfg(feature = "voice_input")]
            ToggleCLIAgentVoiceInput(_) => Empty,
            // Below are actions that are most likely irrelevant to users or are very noisy and the
            // debug version shouldn't be announced.
            Scroll { .. }
            | AltScroll { .. }
            | SharedSessionViewerAltScroll { .. }
            | ClickOnGrid { .. }
            | MaybeDismissToolTip { .. }
            | MaybeClearAltSelect
            | AltScreenContextMenu { .. }
            | AltSelect(_)
            | AltMouseAction(_)
            | ToggleMaximizePane
            | PromptContextMenu { .. }
            | OpenInputContextMenu { .. }
            | InputContextMenuItem(_)
            | NotificationsDiscoveryBanner(_)
            | NotificationsErrorBanner(_)
            | LegacySSHBanner(_)
            | OpenWorkflowModal
            | OpenWorkflowModalForAIWorkflow(_)
            | OpenWorkflowModalForBlock(_)
            | OpenWorkflowModalWithCloudWorkflow(_)
            | OpenShareSessionModal { .. }
            | OpenSharedSessionViewerRoleMenu
            | CopySharedSessionLink { .. }
            | OpenSharedSessionOnDesktop { .. }
            | MakeAllParticipantsReaders { .. }
            | AskAIAssistant { .. }
            | ToggleSnackbarInActivePane
            | SetInputModeAgent
            | SetInputModeTerminal
            | HyperlinkClick { .. }
            | AttemptLoginGatedFeature
            | StartFileDropTarget
            | StopFileDropTarget
            | RunNativeShellCompletions { .. }
            | OpenTeamSettingsPage
            | SelectAgenticSuggestion(_)
            | HideTelemetryBannerPermanently
            | GenerateCodebaseIndex
            | LoadAgentModeConversation
            | DeleteAttachment { .. }
            | OpenAttachmentLightbox { .. }
            | WriteCodebaseIndex
            | ToggleAutoexecuteMode
            | ToggleQueueNextPrompt
            | ToggleTodoPopup
            | CloseTodoPopup
            | ToggleCodeReviewPane { .. }
            | OpenProjectRulesPane
            | InitProject
            | IndexProjectSpeedbump
            | OpenViewMCPPane
            | OpenAddMCPPane
            | OpenBillingAndUsagePane
            | OpenAddRulePane
            | OpenRulesPane
            | OpenEditSkillPane { .. }
            | OpenAddPromptPane
            | AddProjectAtCurrentDirectory
            | CodebaseIndexSpeedbumpBanner(_)
            | AgentModeSetupSpeedbumpBanner(_)
            | AnonymousUserAISignUpBanner(_)
            | SetupCloudEnvironment(_)
            | SetupCloudEnvironmentAndStart(_)
            | TriggerEnvironmentSetupSelection(_)
            | OpenEnvironmentManagementPane
            | DismissCodeToolbeltTooltip
            | SummarizeConversation
            | ToggleLongRunningCommandControl
            | ToggleHideCliResponses
            | OpenConversationsPalette
            | ExitAgentView
            | EnterCloudAgentView
            | StartNewAgentConversation
            | ToggleConversationDetailsPanel
            | CancelAmbientAgentTask
            | OpenInlineHistoryMenu
            | OpenModelSelector
            | ResolvePromptSuggestion(..)
            | AwsBedrockLoginBanner(_)
            | AwsCliNotInstalledBanner(_)
            | ExecuteRewindFromInlineMenu { .. }
            | ToggleUsageFooter
            | RevealChildAgent { .. }
            | SwitchAgentViewToConversation { .. }
            | OpenChildAgentInNewPane { .. }
            | OpenChildAgentInNewTab { .. }
            | StopAgentConversation { .. }
            | KillAgentConversation { .. }
            | ToggleCLIAgentRichInput
            | ToggleSessionRecording => Empty,
        }
    }
}
