use super::*;

impl TerminalView {
    pub(super) fn show_find_bar(&mut self, ctx: &mut ViewContext<Self>) {
        let model = self.model.lock();
        let inverted_blocklist = self.is_inverted_blocklist(ctx);
        // Emit a telemetry event depending on whether the find bar is opened in blocklist or alt screen.
        if model.is_alt_screen_active() {
            send_telemetry_from_ctx!(TelemetryEvent::OpenedAltScreenFind, ctx);
        } else {
            send_telemetry_from_ctx!(
                TelemetryEvent::ContextMenuFindWithinBlocks(self.selected_blocks.cardinality()),
                ctx
            );
        }
        self.find_bar.update(ctx, |view, ctx| {
            let semantic_selection = SemanticSelection::as_ref(ctx);
            if let Some(selected) =
                model.selection_to_string(semantic_selection, inverted_blocklist, ctx)
            {
                if !selected.is_empty() {
                    view.set_query_text(selected.as_str(), ctx);
                }
            }

            // If the alt screen is not active and there are selected blocks, enable the find_within_block.
            view.display_find_within_block = match (
                model.is_alt_screen_active(),
                self.selected_blocks.is_empty(),
            ) {
                (false, false) => FindWithinBlockState::Enabled,
                (false, true) => FindWithinBlockState::Disabled,
                (true, _) => FindWithinBlockState::Hidden,
            };

            ctx.notify();
        });
        drop(model);

        self.find_model.update(ctx, |find_model, _ctx| {
            find_model.set_is_find_bar_open(true);
        });

        let options = self
            .find_model
            .as_ref(ctx)
            .active_find_options()
            .cloned()
            .unwrap_or_default();
        // Start find using the previous query.
        self.run_find(options, ctx);
        self.focus_find_bar(ctx);
    }

    pub(super) fn close_find_bar(&mut self, ctx: &mut ViewContext<Self>) {
        self.find_model.update(ctx, |find_model, _ctx| {
            find_model.set_is_find_bar_open(false);
        });
        ctx.notify();
    }

    pub(super) fn update_find_selection(&mut self, ctx: &mut ViewContext<Self>) {
        if self.find_model.as_ref(ctx).is_find_bar_open()
            && !self.model.lock().is_alt_screen_active()
        {
            let mut find_options = self
                .find_model
                .as_ref(ctx)
                .active_find_options()
                .cloned()
                .unwrap_or_default();

            let new_blocks_to_include_in_results = matches!(
                self.find_bar.as_ref(ctx).display_find_within_block,
                FindWithinBlockState::Enabled
            )
            .then(|| self.selected_blocks.block_indices().collect_vec());

            if find_options.blocks_to_include_in_results.as_ref()
                != new_blocks_to_include_in_results.as_ref()
            {
                self.find_bar.update(ctx, |view, ctx| {
                    if new_blocks_to_include_in_results.is_none() {
                        // If there aren't any selected blocks, turn off find in block
                        view.display_find_within_block = FindWithinBlockState::Disabled;
                    }

                    find_options = find_options
                        .with_blocks_to_include_in_results(new_blocks_to_include_in_results);

                    self.find_model.update(ctx, |find_model, ctx| {
                        find_model.run_find(find_options, ctx)
                    });
                    ctx.notify();
                });
            }
        }
    }

    fn toggle_find_within_block(
        &mut self,
        ctx: &mut ViewContext<Self>,
        enable_find_in_block: bool,
    ) {
        if enable_find_in_block && self.selected_blocks.is_empty() {
            // If a block isn't selected, auto select the most recent block
            self.select_most_recent_blocks(1, ctx);
        } else {
            self.update_find_selection(ctx);
        }
    }

    /// Starts finding the matches for the given query string from the most recent block.
    /// Sets the focused match to the first match in the terminal or doesn't update it.
    /// Note that the meaning of "first" varies depending on whether the block list is inverted
    /// or not.
    fn run_find(&mut self, mut options: FindOptions, ctx: &mut ViewContext<Self>) {
        let blocks_to_include_in_results = matches!(
            self.find_bar.as_ref(ctx).display_find_within_block,
            FindWithinBlockState::Enabled
        )
        .then(|| self.selected_blocks.block_indices());
        options = options.with_blocks_to_include_in_results(blocks_to_include_in_results);

        self.find_model
            .update(ctx, |find_model, ctx| find_model.run_find(options, ctx));

        // Scroll terminal view to the focused match, if any.
        self.scroll_to_match(ctx);

        ctx.notify();
    }

    fn goto_next_find_match(&mut self, direction: &FindDirection, ctx: &mut ViewContext<Self>) {
        self.find_model.update(ctx, |find_model, ctx| {
            find_model.focus_next_find_match(*direction, ctx);
        });
        self.scroll_to_match(ctx);
        ctx.notify();
    }

    pub(super) fn select_most_recent_blocks(&mut self, count: usize, ctx: &mut ViewContext<Self>) {
        if count == 0 {
            self.clear_selected_blocks(ctx);
            return;
        }

        let indices = {
            let terminal_model = self.model.lock();
            (
                terminal_model
                    .block_list()
                    .first_non_hidden_block_by_index(),
                terminal_model.block_list().last_non_hidden_block_by_index(),
            )
        };
        let (Some(first_block_index), Some(last_block_index)) = indices else {
            return;
        };

        let start_index = usize::from(last_block_index)
            .saturating_sub(count - 1)
            .max(usize::from(first_block_index));
        let last_index = usize::from(last_block_index);
        self.change_block_selections(
            |selected_blocks| {
                selected_blocks
                    .reset_to_block_indices((start_index..=last_index).map(BlockIndex::from));
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::BlockSelection(BlockSelectionDetails {
                cardinality: self.selected_blocks.cardinality(),
                delta: BlockSelectionDelta::New,
                is_cmd_down: false,
                is_shift_down: false
            }),
            ctx
        );

        self.tips_completed.update(ctx, |tips, ctx| {
            mark_feature_used_and_write_to_user_defaults(
                Tip::Hint(TipHint::BlockSelect),
                tips,
                ctx,
            );
            ctx.notify();
        });

        // In Agent Mode, block selection is used to attach blocks as context. To allow users to
        // submit queries quickly, we don't want to divert the focus away from the input box. With
        // AgentView enabled, blocks can be attached as context in terminal mode too.
        if !self.ai_input_model.as_ref(ctx).is_ai_input_enabled()
            && !FeatureFlag::AgentView.is_enabled()
        {
            self.focus_terminal(ctx);
        }

        self.scroll_to_if_not_visible(last_block_index, ctx);

        if let Some(accessibility_contents) =
            self.selected_block_accessibility_content(last_block_index)
        {
            ctx.emit_a11y_content(accessibility_contents);
        }
        ctx.notify();
    }

    pub(super) fn select_less_recent_block(
        &mut self,
        is_shift_down: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.is_context_menu_open() {
            self.close_context_menu(ctx, true);
        }

        if let Some(selected_block_index) = self.selected_blocks.tail() {
            let new_block_index = self
                .model
                .lock()
                .block_list()
                .prev_non_hidden_block_from_index(selected_block_index /* from_index */)
                .unwrap_or(selected_block_index);

            if is_shift_down {
                self.change_block_selections(
                    |selected_blocks| {
                        selected_blocks.range_select(new_block_index);
                    },
                    ctx,
                );
            } else {
                self.reset_selection_to_single_block(new_block_index, ctx);
            }

            self.scroll_to_if_not_visible(new_block_index, ctx);
            ctx.notify();

            send_telemetry_from_ctx!(
                TelemetryEvent::BlockSelection(BlockSelectionDetails {
                    delta: BlockSelectionDelta::Previous,
                    is_cmd_down: false,
                    is_shift_down,
                    cardinality: self.selected_blocks.cardinality(),
                }),
                ctx
            );

            self.tips_completed.update(ctx, |tips, ctx| {
                mark_feature_used_and_write_to_user_defaults(
                    Tip::Hint(TipHint::BlockSelect),
                    tips,
                    ctx,
                );
                ctx.notify();
            });
        } else {
            self.select_most_recent_blocks(1, ctx);
        }
    }

    pub(super) fn select_more_recent_block(
        &mut self,
        is_cmd_down: bool,
        is_shift_down: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.is_context_menu_open() {
            self.close_context_menu(ctx, true);
        }
        let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
        let is_inverted_blocklist = input_mode.is_inverted_blocklist();
        let is_most_recent_block_visible = {
            let model = self.model.lock();
            let block_list = model.block_list();
            let viewport = self.viewport_state(block_list, input_mode, ctx);
            if is_inverted_blocklist {
                viewport.is_most_recent_block_in_view(BlockVisibilityMode::TopOfBlockVisible)
            } else {
                viewport.is_most_recent_block_in_view(BlockVisibilityMode::BottomOfBlockVisible)
            }
        };
        let is_long_running_command = {
            self.model
                .lock()
                .block_list()
                .active_block()
                .is_active_and_long_running()
        };
        if let Some(selected_block_index) = self.selected_blocks.tail() {
            let new_block_index = {
                self.model
                    .lock()
                    .block_list()
                    .next_non_hidden_block_from_index(selected_block_index /* from_index */)
                    .unwrap_or(selected_block_index)
            };

            if new_block_index != selected_block_index {
                if is_shift_down {
                    self.change_block_selections(
                        |selected_blocks| {
                            selected_blocks.range_select(new_block_index);
                        },
                        ctx,
                    );
                } else {
                    self.reset_selection_to_single_block(new_block_index, ctx);
                }
                self.scroll_to_if_not_visible(new_block_index, ctx);
                send_telemetry_from_ctx!(
                    TelemetryEvent::BlockSelection(BlockSelectionDetails {
                        cardinality: self.selected_blocks.cardinality(),
                        delta: BlockSelectionDelta::Next,
                        is_cmd_down,
                        is_shift_down,
                    }),
                    ctx
                );
                self.tips_completed.update(ctx, |tips, ctx| {
                    mark_feature_used_and_write_to_user_defaults(
                        Tip::Hint(TipHint::BlockSelect),
                        tips,
                        ctx,
                    );
                    ctx.notify();
                });
            } else if !is_most_recent_block_visible {
                // Scroll to the bottom if the index hasn't changed.
                // This happens when there is a second arrow down when the bottom
                // block is selected.
                self.update_scroll_position_locking(
                    ScrollPositionUpdate::ScrollMostRecentBlockIntoView,
                    ctx,
                );
            } else if is_cmd_down && !is_long_running_command {
                // Focus the input box if the keystroke is cmd-down and we are already at the
                // most recent block (unless it's a long running command, in which case we leave
                // the selection as is.
                self.clear_selected_blocks(ctx);
                ctx.focus(&self.input);
            }
            ctx.notify();
        }
    }

    pub(super) fn select_all_blocks(&mut self, ctx: &mut ViewContext<Self>) {
        let first_block_index = self
            .model
            .lock()
            .block_list()
            .first_non_hidden_block_by_index();
        let last_block_index = self
            .model
            .lock()
            .block_list()
            .last_non_hidden_block_by_index();

        if let Some(start_index) = first_block_index {
            if let Some(end_index) = last_block_index {
                self.change_block_selections(
                    |selected_blocks| {
                        selected_blocks.reset_to_single(start_index);
                        selected_blocks.range_select(end_index);
                    },
                    ctx,
                );
            }
        }
        ctx.focus_self();
        ctx.notify();
    }

    pub(super) fn focus_terminal(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
        ctx.notify();
    }

    pub(super) fn rerender_rich_content_blocks(&mut self, ctx: &mut ViewContext<Self>) {
        for rich_content in self.rich_content_views.iter() {
            if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                ai_metadata
                    .ai_block_handle
                    .update(ctx, |_ai_block, ctx| ctx.notify());
            }
        }
    }

    pub(super) fn reset_selection_to_single_block(
        &mut self,
        block_index: BlockIndex,
        ctx: &mut ViewContext<Self>,
    ) {
        self.change_block_selections(
            |selected_blocks| {
                selected_blocks.reset_to_single(block_index);
            },
            ctx,
        );
        ctx.notify();
    }

    pub(super) fn clear_selected_blocks(&mut self, ctx: &mut ViewContext<Self>) {
        self.change_block_selections(
            |selected_blocks| {
                selected_blocks.reset();
            },
            ctx,
        );
        ctx.notify();
    }

    /// Clears selected text across all types of blocks and handles side effects (i.e. Agent Mode
    /// context, etc.). Never invoke `block_list_mut().clear_selection()` elsewhere on its own.
    pub(super) fn clear_selected_text(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_selected_text_except(None, ctx);
    }

    /// Clears selected text across all types of blocks and handles side effects (i.e. Agent Mode
    /// context, etc.). Never invoke `block_list_mut().clear_selection()` elsewhere on its own.
    ///
    /// Provides the option of keeping the existing text selection for one rich content view, whose
    /// view ID must be passed in via `exempt_rich_content_view_id`. This is helpful for ensuring that
    /// text selections don't simultaneously exist on unrelated views (i.e. a regular command block
    /// and a suggested plan).
    pub(super) fn clear_selected_text_except(
        &mut self,
        exempt_rich_content_view_id: Option<EntityId>,
        ctx: &mut ViewContext<Self>,
    ) {
        // The below function clears all text selections within the underlying `TerminalModel`:
        // - Text selection rendering on regular blocks is tied to the underlying model.
        // - Text selection rendering on rich content blocks is **not** tied to the underlying model.
        //
        // Thus, not only is invoking `clear_selection()` on its own insufficient for clearing all
        // on-screen text selections, but the invocation must also be followed by supplementary logic
        // to clear visual text selections on rich content views.
        //
        // This also explains why we don't invoke this function unless we're attempting to clear
        // **all** selected text; because rich content text copying will stop working otherwise.
        if exempt_rich_content_view_id.is_none() {
            self.model.lock().block_list_mut().clear_selection();
        }

        // Clear all selected text within CLI subagent views,
        // except for the view with a matching view ID.
        for subagent_view in self.cli_subagent_views.values() {
            if exempt_rich_content_view_id.is_some_and(|view_id| subagent_view.id() == view_id) {
                continue;
            }
            subagent_view.update(ctx, |view, ctx| view.clear_all_selections(ctx));
        }

        // Clear all selected text within rich content block view sub-hierarchies,
        // except for the rich content block with a matching view ID.
        for rich_content in self.rich_content_views.iter() {
            match rich_content.metadata() {
                Some(RichContentMetadata::AIBlock(ai_metadata)) => {
                    if exempt_rich_content_view_id
                        .is_some_and(|view_id| ai_metadata.ai_block_handle.id() == view_id)
                    {
                        continue;
                    }
                    ai_metadata
                        .ai_block_handle
                        .update(ctx, |ai_block, ctx| ai_block.clear_all_selections(ctx));
                }
                Some(RichContentMetadata::EnvVarCollectionBlock {
                    env_var_collection_block_handle,
                    ..
                }) => {
                    if exempt_rich_content_view_id
                        .is_some_and(|view_id| env_var_collection_block_handle.id() == view_id)
                    {
                        continue;
                    }
                    env_var_collection_block_handle.update(ctx, |env_var_collection_block, ctx| {
                        env_var_collection_block.clear_selection(ctx);
                    });
                }
                Some(RichContentMetadata::WarpifySuccessBlock { .. }) => {
                    // TODO(Simon): We should be checking for WarpifySuccessBlocks here as well.
                    // The `WarpifySuccessBlock` implements a `SelectableArea`.
                }
                _ => {}
            }
        }

        // When this function is invoked because of an ongoing text selection within a nested
        // rich content view component (i.e. `CodeEditorView`), setting `is_selecting` to false
        // will prevent the selection from "spilling" into neighbouring blocks.
        self.is_selecting = false;

        // TODO(Simon): This doesn't work as intended for nested inline SelectableAreas.
        // This includes inline action headers, requested commands, and env var collection blocks.
        // The reasoning behind this is that `SelectableArea`s don't produce selected text until
        // the selection is **complete**, but `clear_selected_text_except` is only invoked while
        // nested selections are **ongoing**.
        self.maybe_copy_selection_to_clipboard(ctx);
    }

    pub(super) fn clear_selections_when_shell_mode(&mut self, ctx: &mut ViewContext<Self>) {
        // Don't clear selected blocks or text in AI mode because those are context blocks.
        //
        // When `FeatureFlag::AgentView` is enabled, blocks are attachable as AI context in terminal
        // mode. Selections are preserved so they can be attached to the query when entering the
        // agent view.
        if !self.ai_input_model.as_ref(ctx).is_ai_input_enabled()
            && !FeatureFlag::AgentView.is_enabled()
        {
            self.clear_selected_blocks(ctx);
            self.clear_selected_text(ctx);
        }

        self.focus_input_box(ctx);
        ctx.notify();
    }

    pub(super) fn focus_input_and_clear_selections(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_selected_text(ctx);
        self.focus_input_box(ctx);
        ctx.notify();
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub(super) fn clear_selections_when_shell_mode_without_focusing_input(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        // Don't clear selected blocks or text in AI mode because those are context blocks.
        //
        // When `FeatureFlag::AgentView` is enabled, blocks are attachable as AI context in terminal
        // mode. Selections are preserved so they can be attached to the query when entering the
        // agent view.
        if !self.ai_input_model.as_ref(ctx).is_ai_input_enabled()
            && !FeatureFlag::AgentView.is_enabled()
        {
            self.clear_selected_blocks(ctx);
            self.clear_selected_text(ctx);
        }
        ctx.notify();
    }

    pub(super) fn focus_input_box(&mut self, ctx: &mut ViewContext<Self>) {
        // Only clear selected blocks and text if we're not in AI mode since in AI mode we don't want to clear
        // the selected blocks or text (context) when we focus the input.
        //
        // When `FeatureFlag::AgentView` is enabled, blocks are attachable as AI context in terminal
        // mode. Selections are preserved so they can be attached to the query when entering the
        // agent view.
        if !self.ai_render_context.borrow().is_ai_input_enabled
            && !FeatureFlag::AgentView.is_enabled()
        {
            self.clear_selected_blocks(ctx);
        }

        self.update_find_selection(ctx);
        ctx.focus(&self.input);
        ctx.notify();
    }

    fn focus_find_bar(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.find_bar);
        ctx.notify();
    }

    pub(super) fn focus_onboarding_callout_if_active(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let Some(onboarding_callout_view) = self.onboarding_callout_view.as_ref() else {
            return false;
        };

        if !onboarding_callout_view
            .as_ref(ctx)
            .is_onboarding_active(ctx)
        {
            return false;
        }

        ctx.focus(onboarding_callout_view);
        ctx.notify();
        true
    }

    pub(super) fn focus_block_filter_editor(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.block_filter_editor);
        ctx.notify();
    }

    /// Handles AI block events for both live and restored AI blocks.
    pub(super) fn handle_ai_block_event(
        &mut self,
        block: ViewHandle<AIBlock>,
        is_restored: bool,
        event: &AIBlockEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let conversation_id = block.as_ref(ctx).conversation_id();
        match event {
            // -- Live-only events (no-op for restored blocks) ---------------------------
            AIBlockEvent::ActionBlockedOnUserConfirmation => {
                if is_restored {
                    return;
                }
                self.focus_ai_block_if_self_focused(&block, ctx);
                self.maybe_send_agent_mode_desktop_notification(&conversation_id, ctx);
            }
            AIBlockEvent::PassiveCodeDiffLoaded => {
                if is_restored {
                    return;
                }
                self.model
                    .lock()
                    .block_list_mut()
                    .refresh_heights_for_loaded_passive_code_diff(block.id());
                ctx.notify();
            }
            AIBlockEvent::Finished => {
                if is_restored {
                    return;
                }
                // With MAA, it's possible for an exchange to contain many tasks.
                // This means an AI block may "finish" before the entire AI response is complete.
                if self.active_ai_block(ctx).is_none() {
                    self.maybe_send_agent_mode_desktop_notification(&conversation_id, ctx);
                    if self.is_todo_popup_visible {
                        self.is_todo_popup_visible = false;
                    }
                }
                self.redetermine_terminal_focus(ctx);
                ctx.notify();
            }
            AIBlockEvent::OpenCodeWithDiff { view } => {
                if is_restored {
                    return;
                }
                self.open_code_diff(view.clone(), ctx);
                ctx.notify();
            }
            AIBlockEvent::DismissedPassiveBlock => {
                if is_restored {
                    return;
                }
                self.cleanup_and_remove_conversation_for_ai_block(&block, ctx);
            }
            AIBlockEvent::ActionFinished => {
                if is_restored {
                    return;
                }
            }

            // -- Shared events ---------------------------------------------------------
            AIBlockEvent::UpdateInlineActionVisibility {
                action_id,
                is_visible,
            } => {
                self.model
                    .lock()
                    .block_list_mut()
                    .set_visibility_of_block_for_ai_action(action_id, *is_visible);
                if !is_restored {
                    // Requested commands can auto-expand without user interaction so
                    // we only want to take focus if the terminal view is still focused.
                    self.redetermine_terminal_focus(ctx);
                }
                ctx.notify();
            }
            AIBlockEvent::ToggleCodeDiffVisibility => {
                ctx.notify();
            }
            #[cfg(feature = "local_fs")]
            AIBlockEvent::OpenDetectedFilePath {
                absolute_path,
                line_and_column_num,
                target_override,
            } => {
                if let Some(target_override) = target_override {
                    self.open_file_path_with_target(
                        absolute_path.to_path_buf(),
                        target_override.clone(),
                        *line_and_column_num,
                        ctx,
                    );
                } else {
                    self.open_file_path(absolute_path.to_path_buf(), *line_and_column_num, ctx);
                }
            }
            AIBlockEvent::ShowLinkTooltip(tooltip_info) => {
                self.open_rich_content_link_tool_tip = Some(tooltip_info.clone());
            }
            AIBlockEvent::DismissLinkTooltip => {
                self.open_rich_content_link_tool_tip = None;
            }
            AIBlockEvent::ShowSecretTooltip(tooltip_info) => {
                self.open_secret_tool_tip = Some(SecretTooltip::RichContent {
                    is_agent_mode: true,
                    tooltip: tooltip_info.clone(),
                });
            }
            AIBlockEvent::DismissSecretTooltip => {
                self.open_secret_tool_tip = None;
            }
            #[cfg(windows)]
            AIBlockEvent::WindowsCtrlC => {
                self.ctrl_c(ctx);
            }
            AIBlockEvent::AIOutputUpdated => {
                self.model
                    .lock()
                    .block_list_mut()
                    .mark_rich_content_dirty(block.id());
                ctx.notify();
            }
            AIBlockEvent::OpenCitation(citation) => match citation {
                AIAgentCitation::WarpDriveObject { uid } => {
                    ctx.emit(Event::OpenWarpDriveObjectInPane(uid.clone()));
                }
                AIAgentCitation::WarpDocumentation { path } => {
                    ctx.open_url(&format!("https://docs.warp.dev/{path}"));
                }
                AIAgentCitation::WebPage { url } => {
                    ctx.open_url(url);
                }
            },
            AIBlockEvent::OpenAIFactCollection { sync_id } => {
                ctx.emit(Event::OpenAIFactCollection { sync_id: *sync_id });
            }
            AIBlockEvent::OpenWorkflow { sync_id } => {
                if let Some(object) = CloudModel::as_ref(ctx).get_workflow(sync_id) {
                    ctx.emit(Event::OpenWarpDriveObjectInPane(object.uid()));
                }
            }
            AIBlockEvent::OpenSuggestedAgentModeWorkflowModal { workflow_and_id } => {
                ctx.emit(Event::OpenSuggestedAgentModeWorkflowModal {
                    workflow_and_id: workflow_and_id.clone(),
                });
            }
            AIBlockEvent::OpenSuggestedRuleDialog { rule_and_id } => {
                ctx.emit(Event::OpenSuggestedRuleDialog {
                    rule_and_id: rule_and_id.clone(),
                });
            }
            AIBlockEvent::FocusTerminal => {
                self.redetermine_global_focus(ctx);
            }
            AIBlockEvent::ContinueConversation { conversation_id } => {
                self.handle_continue_conversation(conversation_id, ctx);
            }
            AIBlockEvent::ContinuePassiveCodeDiffWithAgent {
                conversation_id,
                trigger_block_id,
                auto_resume,
            } => {
                if let Some(block_id) = trigger_block_id {
                    self.associate_and_promote_block_for_conversation(
                        block_id.clone(),
                        *conversation_id,
                        ctx,
                    );
                }

                if !self
                    .agent_view_controller
                    .as_ref(ctx)
                    .agent_view_state()
                    .is_active()
                {
                    self.set_rich_content_agent_view_conversation_id(block.id(), *conversation_id);
                    self.handle_continue_conversation(conversation_id, ctx);
                }

                if *auto_resume {
                    self.ai_controller.update(ctx, |controller, ctx| {
                        controller.request_follow_up_after_actions(*conversation_id, ctx);
                    });
                }
            }
            AIBlockEvent::ResumeConversation { conversation_id } => {
                self.handle_resume_conversation(conversation_id, ctx);
            }
            AIBlockEvent::InsertForkSlashCommand => {
                self.input.update(ctx, |input, ctx| {
                    input.replace_buffer_content(&format!("{} ", commands::FORK.name), ctx);
                    ctx.focus_self();
                });
            }
            AIBlockEvent::ChildViewTextSelected => {
                self.clear_selected_text_except(Some(block.id()), ctx);
            }
            AIBlockEvent::CopiedEmptyText => {
                self.copy(ctx);
            }
            AIBlockEvent::UsageFooterToggled {
                conversation_id,
                is_expanded,
            } => {
                self.handle_usage_footer_toggled(block.id(), *conversation_id, *is_expanded, ctx);
            }
            AIBlockEvent::OpenSettings => {
                ctx.emit(Event::OpenSettings(SettingsSection::WarpAgent));
            }
            #[cfg(feature = "local_fs")]
            AIBlockEvent::OpenCodeInWarp { source, layout } => {
                ctx.emit(Event::OpenCodeInWarp {
                    source: source.clone(),
                    layout: *layout,
                });
            }
            AIBlockEvent::ToggleCodeReviewPane { entrypoint } => {
                self.toggle_code_review_pane(
                    GitDeltaPreference::Always,
                    *entrypoint,
                    None, // cli_agent
                    true, /* focus_new_pane */
                    ctx,
                );
            }
            AIBlockEvent::OpenImportedCommentInCodeReview {
                repo_path,
                comment,
                base_branch,
            } => {
                let arg = CodeReviewPanelArg {
                    repo_path: Some(repo_path.clone()),
                    terminal_view: self.view_handle.clone(),
                    entrypoint: CodeReviewPaneEntrypoint::AgentModeRunning,
                    focus_new_pane: true,
                    cli_agent: None,
                };
                let diff_mode = self.diff_mode_for_branch(base_branch.as_deref(), ctx);
                ctx.emit(Event::OpenCodeReviewPaneAndScrollToComment {
                    open_code_review: arg,
                    comment: (**comment).clone(),
                    diff_mode,
                });
            }
            AIBlockEvent::OpenAllImportedCommentsForConversation { conversation_id } => {
                let (all_comments, base_branch) = self.all_comments_in_thread(conversation_id, ctx);
                if !all_comments.is_empty() {
                    let diff_mode = self.diff_mode_for_branch(base_branch.as_deref(), ctx);
                    ctx.emit(Event::ImportAllCodeReviewComments {
                        open_code_review: self.imported_comments_panel_arg(),
                        comments: all_comments,
                        diff_mode,
                    });
                }
            }
            AIBlockEvent::OpenAIDocumentPane {
                document_id,
                document_version,
                is_auto_open,
            } => {
                ctx.emit(Event::OpenAIDocumentPane {
                    document_id: *document_id,
                    document_version: *document_version,
                    is_auto_open: *is_auto_open,
                });
            }
            AIBlockEvent::OpenActiveAgentProfileEditor => {
                let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
                let active_profile = profiles_model.active_profile(Some(self.view_id), ctx);
                ctx.emit(Event::OpenAgentProfileEditor {
                    profile_id: *active_profile.id(),
                });
            }
            AIBlockEvent::OpenThemeChooser => {
                ctx.emit(Event::OpenThemeChooser);
            }
            AIBlockEvent::RunAwsLoginCommand => {
                self.run_aws_login_command(ctx);
            }
        }
        ctx.notify();
    }

    fn imported_comments_panel_arg(&self) -> CodeReviewPanelArg {
        CodeReviewPanelArg {
            repo_path: self.current_repo_path.clone(),
            terminal_view: self.view_handle.clone(),
            entrypoint: CodeReviewPaneEntrypoint::AgentModeRunning,
            focus_new_pane: true,
            cli_agent: None,
        }
    }

    /// Returns the exchange ID of the most recent user-query exchange in the
    /// given conversation, which marks the start of the current thread.
    ///
    /// Returns `None` if the conversation has no user-query exchanges.
    fn thread_start_exchange_id(
        conversation_id: &AIConversationId,
        ctx: &AppContext,
    ) -> Option<AIAgentExchangeId> {
        BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(conversation_id)
            .and_then(|conv| {
                conv.exchanges_reversed()
                    .find(|exchange| exchange.has_user_query())
                    .map(|exchange| exchange.id)
            })
    }

    /// Returns an iterator over the `AIBlockMetadata` entries that belong to the
    /// current thread of `conversation_id` (newest first, bounded by the most
    /// recent user query).
    ///
    /// This does **not** dereference view handles; callers add their own
    /// `.map()` to obtain `&AIBlock` references.
    fn ai_block_metadata_for_current_thread<'a>(
        &'a self,
        conversation_id: &'a AIConversationId,
        ctx: &'a AppContext,
    ) -> impl Iterator<Item = &'a AIBlockMetadata> + 'a {
        let thread_start_exchange_id = Self::thread_start_exchange_id(conversation_id, ctx);

        self.rich_content_views
            .iter()
            .rev()
            .filter_map(move |rc| {
                let ai_metadata = rc.ai_block_metadata()?;
                (ai_metadata.conversation_id == *conversation_id).then_some(ai_metadata)
            })
            .take_while_inclusive(move |ai_metadata| {
                Some(ai_metadata.exchange_id) != thread_start_exchange_id
            })
    }

    /// Returns an iterator over the `AIBlock`s that belong to the current
    /// thread of `conversation_id` (newest first, bounded by the most recent
    /// user query).
    fn ai_blocks_for_current_thread<'a>(
        &'a self,
        conversation_id: &'a AIConversationId,
        ctx: &'a AppContext,
    ) -> impl Iterator<Item = &'a AIBlock> + 'a {
        self.ai_block_metadata_for_current_thread(conversation_id, ctx)
            .map(move |ai_metadata| ai_metadata.ai_block_handle.as_ref(ctx))
    }

    /// Collects all imported review comments from blocks in the current thread of the given
    /// conversation.
    fn all_comments_in_thread(
        &self,
        conversation_id: &AIConversationId,
        ctx: &AppContext,
    ) -> (Vec<AttachedReviewComment>, Option<String>) {
        let mut all_comments = Vec::new();
        let mut base_branch = None;
        for ai_block in self.ai_blocks_for_current_thread(conversation_id, ctx) {
            if let Some(imported) = ai_block.collect_imported_comments() {
                all_comments.extend(imported.comments);
                if base_branch.is_none() {
                    base_branch = imported.base_branch;
                }
            }
        }
        // The iterator yields blocks newest-first; reverse to get chronological order.
        all_comments.reverse();
        (all_comments, base_branch)
    }

    /// Returns `true` if any block in the current thread of the given conversation has imported
    /// review comments.
    pub(crate) fn has_imported_comments_in_thread(
        &self,
        conversation_id: &AIConversationId,
        ctx: &AppContext,
    ) -> bool {
        self.ai_blocks_for_current_thread(conversation_id, ctx)
            .any(|ai_block| ai_block.has_any_imported_comments())
    }

    pub(super) fn active_ai_block(&self, ctx: &AppContext) -> Option<&ViewHandle<AIBlock>> {
        // Skip trailing non-AI items (usage footers) as they don't impact the conversation state.
        let candidate = self
            .rich_content_views
            .iter()
            .rev()
            .find(|rc| !rc.is_usage_footer() && !rc.is_pending_user_query());

        candidate.and_then(|rich_content| {
            let ai_metadata = rich_content.ai_block_metadata()?;
            let ai_block = ai_metadata.ai_block_handle.as_ref(ctx);

            (!ai_block.is_finished()
                && !ai_block.is_hidden(ctx)
                && !ai_block.is_passive_conversation(ctx))
            .then_some(&ai_metadata.ai_block_handle)
        })
    }

    /// Check if there's an active (non-completed, non-cancelled) /init in progress
    pub(super) fn has_active_init_project(&self, ctx: &AppContext) -> bool {
        self.active_init_project_model
            .as_ref()
            .is_some_and(|model| model.as_ref(ctx).is_active())
    }

    /// Check if there are any init step blocks for the given conversation
    pub(super) fn has_init_steps_for_conversation(
        &self,
        conversation_id: AIConversationId,
    ) -> bool {
        self.rich_content_views
            .iter()
            .any(|rc| rc.is_init_step() && rc.agent_view_conversation_id() == Some(conversation_id))
    }

    /// Returns whether the last block in the currently visible conversation is an `InitStepBlock`.
    pub(super) fn is_last_block_init_step(&self, ctx: &AppContext) -> bool {
        let last_visible_block = if FeatureFlag::AgentView.is_enabled() {
            let visible_conversation_id = self
                .agent_view_controller
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id();
            self.rich_content_views
                .iter()
                .rev()
                .find(|rc| rc.agent_view_conversation_id() == visible_conversation_id)
        } else {
            self.rich_content_views.last()
        };

        last_visible_block.is_some_and(|rc| rc.is_init_step())
    }

    /// Returns the last block's `InitEnvironmentBlock` if it is uncompleted, scoped to the
    /// currently visible conversation.
    pub(super) fn active_init_environment_block(
        &self,
        ctx: &AppContext,
    ) -> Option<&ViewHandle<InitEnvironmentBlock>> {
        let last_visible_block = if FeatureFlag::AgentView.is_enabled() {
            let visible_conversation_id = self
                .agent_view_controller
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id();
            self.rich_content_views
                .iter()
                .rev()
                .find(|rc| rc.agent_view_conversation_id() == visible_conversation_id)
        } else {
            self.rich_content_views.last()
        }?;

        if let Some(RichContentMetadata::InitEnvironment { block_handle }) =
            last_visible_block.metadata()
        {
            return (!block_handle.as_ref(ctx).completed()).then_some(block_handle);
        }
        None
    }

    pub(super) fn ai_block_for_exchange(
        &self,
        exchange_id: &AIAgentExchangeId,
    ) -> Option<&ViewHandle<AIBlock>> {
        self.rich_content_views.iter().find_map(|rich_content| {
            let ai_metadata = rich_content.ai_block_metadata()?;
            if ai_metadata.exchange_id == *exchange_id {
                return Some(&ai_metadata.ai_block_handle);
            }
            None
        })
    }

    pub(super) fn ai_block_handle_by_view_id(
        &self,
        view_id: EntityId,
    ) -> Option<&ViewHandle<AIBlock>> {
        self.rich_content_views.iter().find_map(|rich_content| {
            let ai_metadata = rich_content.ai_block_metadata()?;
            if ai_metadata.ai_block_handle.id() == view_id {
                return Some(&ai_metadata.ai_block_handle);
            }
            None
        })
    }

    /// Returns the last block's `EnvVarCollectionBlock` if it is uncompleted, scoped to the
    /// currently visible conversation.
    pub(super) fn active_env_var_collection_block(
        &self,
        ctx: &AppContext,
    ) -> Option<&ViewHandle<EnvVarCollectionBlock>> {
        if FeatureFlag::AgentView.is_enabled() {
            let visible_conversation_id = self
                .agent_view_controller
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id();
            let last_visible_block = self
                .rich_content_views
                .iter()
                .rev()
                .find(|rc| rc.agent_view_conversation_id() == visible_conversation_id)?;

            if let Some(RichContentMetadata::EnvVarCollectionBlock {
                env_var_collection_block_handle,
            }) = last_visible_block.metadata()
            {
                return (!env_var_collection_block_handle
                    .as_ref(ctx)
                    .is_block_completed())
                .then_some(env_var_collection_block_handle);
            }
            None
        } else {
            self.rich_content_views.iter().find_map(|rich_content| {
                if let Some(RichContentMetadata::EnvVarCollectionBlock {
                    env_var_collection_block_handle,
                }) = rich_content.metadata()
                {
                    return (!env_var_collection_block_handle
                        .as_ref(ctx)
                        .is_block_completed())
                    .then_some(env_var_collection_block_handle);
                }
                None
            })
        }
    }

    /// Examines the local state of the [`TerminalView`] and chooses where best to assign focus.
    ///
    /// WARNING: this can steal focus even when the user is working in a separate terminal view!
    /// Consider using [`Self::redetermine_terminal_focus`] instead.
    ///
    /// WARNING: this method takes a lock on the TerminalModel.
    /// Caller must ensure the model is not already locked!
    ///
    /// TODO: https://linear.app/warpdotdev/issue/CORE-277
    pub fn redetermine_global_focus(&mut self, ctx: &mut ViewContext<Self>) {
        if self.context_menu_state.is_some() {
            // This is a hack to avoid focusing on the terminal which
            // calls on_blur and closes the context menu when it is supposed
            // to open after closing the command palette
            // TODO: refactor in the future
            return;
        }

        if OneTimeModalModel::as_ref(ctx).is_any_modal_open() {
            return;
        }

        // If the onboarding callout is active, it should win focus so that its displayed
        // keybindings (enter/delete) actually work.
        if self.focus_onboarding_callout_if_active(ctx) {
            return;
        }

        self.last_focus_ts = Some(Local::now().naive_local());

        let is_input_visible = {
            let model = self.model.lock();
            self.is_input_box_visible(&model, ctx)
        };
        let should_focus_terminal = {
            let semantic_selection = SemanticSelection::as_ref(ctx);
            let model = self.model.lock();
            let block_list = model.block_list();

            let has_bootstrapped = model.block_list().is_bootstrapping_precmd_done();

            let has_active_user_terminal_command = block_list.active_block().is_active_and_long_running()
                && !block_list.active_block().is_agent_in_control()
                // The only case where terminal can take focus _while_ input is visible is
                // pre-bootstrap, for example when oh-my-zsh prompts you to update -- at this point
                // the input is visible but you should still be able to click into the block for the
                // oh-my-zsh prompt and send input directly to the pty.
                && (!is_input_visible || !has_bootstrapped);

            let is_shell_mode = !self.ai_input_model.as_ref(ctx).is_ai_input_enabled();
            let are_blocks_selected = !self.selected_blocks.is_empty();
            let is_text_selected = model
                .selection_to_string(semantic_selection, false, ctx)
                .filter(|text| !text.is_empty())
                .is_some();

            // Leave the input box focused when selecting blocks or text as context in AI input
            // mode so users can quickly submit queries.
            //
            // In the new modality, block selection always represents context attachment and the
            // input should remain focused.
            let has_block_or_text_selection_in_shell_mode = is_shell_mode
                && !FeatureFlag::AgentView.is_enabled()
                && (are_blocks_selected || is_text_selected);

            has_active_user_terminal_command || has_block_or_text_selection_in_shell_mode
        };
        let blocked_cli_subagent_view = {
            let model = self.model.lock();
            let active_block = model.block_list().active_block();
            if active_block.is_agent_blocked() {
                self.cli_subagent_views.get(active_block.id())
            } else {
                None
            }
        };

        if let Some(blocked_cli_subagent_view) = blocked_cli_subagent_view {
            ctx.focus(blocked_cli_subagent_view);
        } else if should_focus_terminal {
            self.focus_terminal(ctx);
        } else if let Some(ssh_choice_view) = self.active_ssh_remote_server_choice_block() {
            ctx.focus(&ssh_choice_view);
        } else if let (Some(active_ai_block_view_handle), false) =
            (self.active_ai_block(ctx), is_input_visible)
        {
            ctx.focus(active_ai_block_view_handle);
        } else if self.has_active_init_project(ctx) && self.is_last_block_init_step(ctx) {
            self.try_focus_active_init_step(ctx);
        } else if let Some(active_init_environment_block_handle) =
            self.active_init_environment_block(ctx)
        {
            active_init_environment_block_handle
                .update(ctx, |block, ctx| block.try_steal_focus(ctx));
        } else if let Some(env_var_collection_block_handle) =
            self.active_env_var_collection_block(ctx)
        {
            ctx.focus(env_var_collection_block_handle);
        } else {
            self.focus_input_box(ctx);
        }
    }

    pub(super) fn close_context_menu(
        &mut self,
        ctx: &mut ViewContext<Self>,
        should_redetermine_focus: bool,
    ) {
        if self.context_menu_state.is_some() {
            self.context_menu_state = None;
            ctx.notify();
            if should_redetermine_focus {
                self.redetermine_global_focus(ctx);
            }
        }
    }

    pub(super) fn context_menu_insert_selected_text(&mut self, ctx: &mut ViewContext<Self>) {
        {
            send_telemetry_from_ctx!(TelemetryEvent::ContextMenuInsertSelectedText, ctx);
            let semantic_selection = SemanticSelection::as_ref(ctx);
            // Note: we purposely separate this expression here, to avoid locking the TerminalModel for the duration of the `if let`
            // block, since downstream functions may need the lock (`Input::insert_internal`).
            let selected_text = self.model.lock().selection_to_string(
                semantic_selection,
                self.is_inverted_blocklist(ctx),
                ctx,
            );
            if let Some(selected_text) = selected_text {
                // We put everything from the selection into the input box, even
                // if it includes non-printable characters. Note that this is
                // important to handle new lines appropriately.
                self.input.update(ctx, |input, ctx| {
                    input.system_insert(&selected_text, ctx);
                    ctx.focus_self();
                })
            }
        }
        self.close_context_menu(ctx, true);
    }

    fn input_command(&mut self, ctx: &mut ViewContext<Self>, command: String) {
        send_telemetry_from_ctx!(
            TelemetryEvent::ReinputCommands(self.selected_blocks.cardinality()),
            ctx
        );
        self.input.update(ctx, |input, ctx| {
            input.replace_buffer_content((command).trim(), ctx);
            ctx.focus_self();
        });
    }

    pub(super) fn reinput_commands(&mut self, as_root: bool, ctx: &mut ViewContext<Self>) {
        if !self.selected_blocks.is_empty() {
            let mut commands = vec![];
            self.with_non_hidden_selected_blocks(
                |block| {
                    let command_str = block.command_to_string();
                    if !command_str.trim().is_empty() {
                        if as_root {
                            commands.push(format!("sudo {command_str}"));
                        } else {
                            commands.push(command_str);
                        }
                    }
                },
                ctx,
            );
            self.input_command(ctx, commands.join("\n"));
            self.focus_input_box(ctx);
        }
    }

    pub(super) fn context_menu_open_share_block_modal(
        &mut self,
        block_index: BlockIndex,
        ctx: &mut ViewContext<Self>,
    ) {
        if AuthStateProvider::as_ref(ctx)
            .get()
            .is_anonymous_or_logged_out()
        {
            AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                auth_manager.attempt_login_gated_feature(
                    "Share Block",
                    AuthViewVariant::ShareRequirementCloseable,
                    ctx,
                )
            });
            return;
        }

        send_telemetry_from_ctx!(
            TelemetryEvent::ContextMenuOpenShareModal(self.selected_blocks.cardinality()),
            ctx
        );
        self.tips_completed.update(ctx, |tips, ctx| {
            mark_feature_used_and_write_to_user_defaults(
                Tip::Hint(TipHint::BlockAction),
                tips,
                ctx,
            );
            ctx.notify();
        });
        ctx.emit(Event::ShareModalOpened(block_index));
        self.close_context_menu(ctx, true);
        ctx.notify();
    }

    pub(super) fn open_share_block_modal(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(selected_index) = self.selected_blocks.tail() {
            self.context_menu_open_share_block_modal(selected_index, ctx);
        }
    }

    pub(super) fn context_menu_copy_blocks(&mut self, ctx: &mut ViewContext<Self>) {
        self.copy_blocks(BlockEntity::CommandAndOutput, ctx);
    }

    pub(super) fn context_menu_copy_block_commands(&mut self, ctx: &mut ViewContext<Self>) {
        self.copy_blocks(BlockEntity::Command, ctx);
    }

    pub(super) fn context_menu_copy_block_outputs(&mut self, ctx: &mut ViewContext<Self>) {
        self.copy_blocks(BlockEntity::Output, ctx);
    }

    pub(super) fn context_menu_copy_filtered_block_outputs(&mut self, ctx: &mut ViewContext<Self>) {
        self.copy_blocks(BlockEntity::FilteredOutput, ctx);
    }

    pub(super) fn context_menu_copy_url(&mut self, url_content: &str, ctx: &mut ViewContext<Self>) {
        ctx.clipboard()
            .write(ClipboardContent::plain_text(url_content.to_string()));
        self.close_context_menu(ctx, true);
    }

    pub(super) fn num_non_hidden_selected_blocks(&self) -> usize {
        let model = self.model.lock();
        let agent_view_state = model.block_list().agent_view_state();
        self.selected_blocks
            .ranges()
            .iter()
            .flat_map(|range| range.range(None))
            .filter(|block_index| {
                model
                    .block_list()
                    .block_at(*block_index)
                    .is_some_and(|block| !block.is_empty(agent_view_state))
            })
            .count()
    }

    pub(super) fn with_non_hidden_selected_blocks<T>(
        &mut self,
        mut action: T,
        ctx: &mut ViewContext<Self>,
    ) where
        T: FnMut(&Block),
    {
        let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
        let sort_direction = input_mode.block_sort_direction();
        let model = self.model.lock();
        let agent_view_state = model.block_list().agent_view_state();
        let sorted_ranges = self.selected_blocks.sorted_ranges(sort_direction);
        for selection_range in sorted_ranges {
            for block_index in selection_range.range(Some(sort_direction)) {
                if let Some(block) = model
                    .block_list()
                    .block_at(block_index)
                    .filter(|block| !block.is_empty(agent_view_state))
                {
                    action(block);
                }
            }
        }
    }

    pub(super) fn copy_blocks(&mut self, entity: BlockEntity, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(
            TelemetryEvent::ContextMenuCopy(entity, self.selected_blocks.cardinality()),
            ctx
        );
        self.tips_completed.update(ctx, |tips, ctx| {
            mark_feature_used_and_write_to_user_defaults(
                Tip::Hint(TipHint::BlockAction),
                tips,
                ctx,
            );
            ctx.notify();
        });

        let selected_block_contents = self.selected_block_contents_as_string(entity, "\n", ctx);
        ctx.clipboard()
            .write(ClipboardContent::plain_text(selected_block_contents));
        self.close_context_menu(ctx, true);
    }

    pub(super) fn context_menu_copy_selected_text(&mut self, ctx: &mut ViewContext<Self>) {
        {
            send_telemetry_from_ctx!(TelemetryEvent::ContextMenuCopySelectedText, ctx);
            let semantic_selection = SemanticSelection::as_ref(ctx);
            let model = self.model.lock();
            let selected_text =
                model.selection_to_string(semantic_selection, self.is_inverted_blocklist(ctx), ctx);
            if let Some(selected_text) = selected_text {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(selected_text));
            }
        }
        self.close_context_menu(ctx, true);
    }

    pub(super) fn selected_block_contents_as_string(
        &mut self,
        entity: BlockEntity,
        separator: &str,
        ctx: &mut ViewContext<Self>,
    ) -> String {
        let mut block_strs = vec![];
        self.with_non_hidden_selected_blocks(
            |block| {
                let block_str = match entity {
                    BlockEntity::Command => block.command_to_string(),
                    BlockEntity::Output => block.output_to_string_force_full_grid_contents(),
                    BlockEntity::CommandAndOutput => format!(
                        "{}\n{}",
                        block.command_to_string(),
                        block.output_to_string(),
                    ),
                    BlockEntity::FilteredOutput => block.output_to_string(),
                };

                if !block_str.trim().is_empty() {
                    block_strs.push(block_str);
                }
            },
            ctx,
        );

        block_strs.join(separator)
    }

    pub(super) fn handle_menu_event(&mut self, event: &MenuEvent, ctx: &mut ViewContext<Self>) {
        if let MenuEvent::Close { via_select_item } = event {
            self.close_context_menu(ctx, !*via_select_item);
        }
    }

    pub(super) fn bookmark_selected_block(&mut self, ctx: &mut ViewContext<Self>) {
        self.tips_completed.update(ctx, |tips, ctx| {
            mark_feature_used_and_write_to_user_defaults(
                Tip::Hint(TipHint::BlockAction),
                tips,
                ctx,
            );
            ctx.notify();
        });
        if let Some(selected_block_index) = self.selected_blocks.tail() {
            self.bookmark_block(&selected_block_index, ctx);
            ctx.notify();
        }
    }

    pub(super) fn bookmark_block(&mut self, index: &BlockIndex, ctx: &mut ViewContext<Self>) {
        let enable_bookmark = match self.bookmarked_blocks.entry(*index) {
            Entry::Occupied(occupied) => {
                occupied.remove();
                false
            }
            Entry::Vacant(vacant) => {
                vacant.insert(Default::default());
                true
            }
        };

        send_telemetry_from_ctx!(
            TelemetryEvent::BookmarkBlockToggled { enable_bookmark },
            ctx
        );

        ctx.notify();
    }

    pub(super) fn is_navigated_away_from_window(&self, ctx: &mut ViewContext<Self>) -> bool {
        let active_window = ctx.windows().active_window();
        Some(ctx.window_id()) != active_window
    }

    fn is_block_active_and_running(&self, model: &TerminalModel, block_index: BlockIndex) -> bool {
        let active_block = model.block_list().active_block();
        active_block.index() == block_index && active_block.is_active_and_long_running()
    }

    pub fn has_active_long_running_command(&self) -> bool {
        let model = self.model.lock();
        model
            .block_list()
            .active_block()
            .is_active_and_long_running()
    }

    /// If password notification settings enabled, send a notification.
    /// Otherwise, set the banner trigger so that we show the banner the next
    /// time a block completes.
    pub fn maybe_send_password_notification(
        &mut self,
        block_index: BlockIndex,
        ctx: &mut ViewContext<Self>,
    ) {
        let model = self.model.lock();
        let active_block = model.block_list().active_block();
        let notification_settings = SessionSettings::as_ref(ctx).notifications.value().clone();

        // The active block could have changed before we send the notification
        // so double check before sending
        if self.is_block_active_and_running(&model, block_index) {
            match notification_settings.mode {
                NotificationsMode::Enabled if notification_settings.is_needs_attention_enabled => {
                    let password_trigger = NotificationsTrigger::NeedsAttention;
                    let notification_content = password_trigger.create_notification_content(
                        active_block.command_to_string(),
                        "Command is waiting for a password".to_string(),
                    );
                    ctx.emit(Event::SendNotification(notification_content));
                    send_telemetry_from_ctx!(
                        TelemetryEvent::NotificationSent {
                            trigger: password_trigger,
                            agent_variant: None,
                        },
                        ctx
                    );
                }
                NotificationsMode::Unset
                    if matches!(
                        self.inline_banners_state.notifications_discovery_banner,
                        NotificationsDiscoveryBanner::Unset
                    ) =>
                {
                    // if the user hasn't configured notifications before and there isn't already
                    // a banner, we should add the banner once the block completes
                    self.inline_banners_state.notifications_discovery_banner =
                        NotificationsDiscoveryBanner::Triggered(
                            NotificationsTrigger::NeedsAttention,
                        );
                }
                _ => {}
            }
        }
    }

    fn restore_followup_prompt_after_failed_submission(
        &mut self,
        prompt: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        self.pending_cloud_followup_task_id = None;
        self.input.update(ctx, |input, ctx| {
            input.reset_after_cloud_followup_submission(ctx);
            input.replace_buffer_content(prompt, ctx);
            input.set_input_mode_agent(true, ctx);
        });
        self.update_pane_configuration(ctx);
        self.focus_input_box(ctx);
        ctx.notify();
    }

    pub(super) fn try_submit_pending_cloud_followup(
        &mut self,
        prompt: String,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if !FeatureFlag::HandoffCloudCloud.is_enabled() {
            return false;
        }
        let Some(task_id) = self
            .pending_cloud_followup_task_id
            .or_else(|| self.owned_ambient_agent_task_id(ctx))
        else {
            return false;
        };

        if prompt.trim().is_empty() {
            self.input.update(ctx, |input, ctx| {
                input.reset_after_cloud_followup_submission(ctx);
                input.set_input_mode_agent(true, ctx);
            });
            self.update_pane_configuration(ctx);
            self.focus_input_box(ctx);
            ctx.notify();
            return true;
        }

        let Some(ambient_agent_view_model) = self.ambient_agent_view_model.clone() else {
            self.restore_followup_prompt_after_failed_submission(&prompt, ctx);
            self.show_error_toast("Couldn't continue this cloud task.".to_string(), ctx);
            return true;
        };

        if ambient_agent_view_model.as_ref(ctx).task_id() != Some(task_id) {
            self.restore_followup_prompt_after_failed_submission(&prompt, ctx);
            self.show_error_toast("Couldn't continue this cloud task.".to_string(), ctx);
            return true;
        }

        ambient_agent_view_model.update(ctx, |model, ctx| {
            model.submit_cloud_followup(prompt, ctx);
        });
        self.input.update(ctx, |input, ctx| {
            input.reset_after_cloud_followup_submission(ctx);
            input.set_input_mode_agent(true, ctx);
        });
        self.update_pane_configuration(ctx);
        ctx.notify();
        true
    }

    pub(super) fn handle_input_event(&mut self, event: &InputEvent, ctx: &mut ViewContext<Self>) {
        match event {
            InputEvent::Enter => self.clear_prompt_suggestions(ctx),
            InputEvent::PageUp => self.page_up(ctx),
            InputEvent::PageDown => self.page_down(ctx),
            InputEvent::ExecuteCommand(event) => {
                self.update_scroll_position_locking(
                    ScrollPositionUpdate::AfterCommandExecutionStarted,
                    ctx,
                );
                if let Some(active_session) = self
                    .active_block_session_id()
                    .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id))
                {
                    active_session.cancel_active_commands();
                }

                // Don't steal focus from other parts of the app.
                if ctx.is_self_or_child_focused() {
                    self.focus_terminal(ctx);
                }

                ctx.emit(Event::ExecuteCommand(event.as_ref().clone()));

                if self.block_onboarding_active {
                    self.interrupt_onboarding_blocks(ctx);
                }
            }
            InputEvent::ExecuteAIQuery => {
                // Clear the "enter again to send" ephemeral message if it's currently showing
                self.ephemeral_message_model.update(ctx, |model, ctx| {
                    if model
                        .current_message()
                        .and_then(|msg| msg.id())
                        .is_some_and(|id| id == agent_view::ENTER_AGAIN_TO_SEND_MESSAGE_ID)
                    {
                        model.clear_message(ctx);
                    }
                });

                // For scrolling purposes, treat executing an AI query as executing a command. We'll
                // also update the scroll position when the rich AI content block is added, but that
                // has different scroll behavior.
                self.update_scroll_position_locking(
                    ScrollPositionUpdate::AfterCommandExecutionStarted,
                    ctx,
                );
            }
            InputEvent::SendAgentPrompt {
                server_conversation_token,
                prompt,
                attachments,
            } => {
                ctx.emit(Event::SendAgentPrompt {
                    server_conversation_token: *server_conversation_token,
                    prompt: prompt.clone(),
                    attachments: attachments.clone(),
                });
            }
            InputEvent::SubmitCloudFollowup { prompt } => {
                if FeatureFlag::HandoffCloudCloud.is_enabled()
                    && self.try_submit_pending_cloud_followup(prompt.clone(), ctx)
                {
                    return;
                }
                self.show_error_toast("Couldn't continue this cloud task.".to_string(), ctx);
            }
            InputEvent::CancelSharedSessionConversation {
                server_conversation_token,
            } => {
                ctx.emit(Event::CancelSharedSessionConversation {
                    server_conversation_token: *server_conversation_token,
                });
            }
            InputEvent::ClearSelectedBlock => self.clear_selected_blocks(ctx),
            InputEvent::SelectRecentBlocks { count } => {
                let is_first_selection = self.selected_blocks.is_empty();
                if is_first_selection && self.ai_input_model.as_ref(ctx).is_ai_input_enabled() {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::AgentModeAttachedBlockContext {
                            method: AgentModeAttachContextMethod::Keyboard
                        },
                        ctx
                    );
                }
                self.select_most_recent_blocks(*count, ctx)
            }
            InputEvent::Copy => self.copy(ctx),
            InputEvent::UnhandledModifierKeyOnEditor(keystroke) => {
                send_telemetry_from_ctx!(
                    TelemetryEvent::EditorUnhandledModifierKey(keystroke.as_ref().to_owned()),
                    ctx
                );
            }
            InputEvent::ClearSelectionsWhenShellMode => self.clear_selections_when_shell_mode(ctx),
            InputEvent::AutosuggestionAccepted => {
                // TODO(suraj): maybe pass down the autosuggestion type and send
                // the telemetry deeper so we don't have to guesstimate the state
                if let Some(most_recent_command_correction) =
                    self.most_recent_command_correction.as_ref()
                {
                    let buffer_text = self.input.as_ref(ctx).buffer_text(ctx);
                    if buffer_text == most_recent_command_correction.command {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::CommandCorrection {
                                event: CommandCorrectionEvent::Accepted {
                                    via: CommandCorrectionAcceptedType::Autosuggestion,
                                    rule: most_recent_command_correction.rule_applied.to_str(),
                                }
                            },
                            ctx
                        );
                    }
                }
                // When an AI query autosuggestion is accepted, there might be attached context
                // blocks we need to render the border for.
                ctx.notify()
            }
            InputEvent::UnhandledCmdEnter => {
                if is_accept_prompt_suggestion_bound_to_cmd_enter(ctx) {
                    self.resolve_passive_suggestion(
                        PromptSuggestionResolution::Accept {
                            interaction_source: InteractionSource::Keybinding,
                        },
                        ctx,
                    );
                }
            }
            InputEvent::CtrlEnter => {
                if is_accept_prompt_suggestion_bound_to_ctrl_enter(ctx) {
                    self.resolve_passive_suggestion(
                        PromptSuggestionResolution::Accept {
                            interaction_source: InteractionSource::Keybinding,
                        },
                        ctx,
                    );
                }
            }
            InputEvent::EnterAgentView {
                initial_prompt,
                conversation_id,
                origin,
            } => match conversation_id {
                Some(id) => {
                    self.enter_agent_view_for_conversation(
                        initial_prompt.clone(),
                        *origin,
                        *id,
                        ctx,
                    );
                }
                None => {
                    self.enter_agent_view_for_new_conversation(
                        initial_prompt.clone(),
                        *origin,
                        ctx,
                    );
                }
            },
            InputEvent::EnterCloudAgentView { initial_prompt } => {
                self.enter_cloud_agent_view(initial_prompt.clone(), ctx);
            }
            InputEvent::CreateDockerSandbox => {
                if !FeatureFlag::LocalDockerSandbox.is_enabled() {
                    log::warn!("Local docker sandbox feature flag is disabled");
                    return;
                }
                self.create_and_push_docker_sandbox(ctx);
            }
            InputEvent::ExitCloudModeAndStartLocalAgent { initial_prompt } => {
                let origin = AgentViewEntryOrigin::Input {
                    was_prompt_autodetected: false,
                };
                let initial_prompt = initial_prompt.clone();

                if let Some(pane_stack) = self.pane_stack.as_ref().and_then(|h| h.upgrade(ctx)) {
                    let should_pop = pane_stack.as_ref(ctx).depth() > 1;
                    if should_pop {
                        pane_stack.update(ctx, |stack, ctx| {
                            stack.pop(ctx);
                        });
                    }

                    let active_view = pane_stack.as_ref(ctx).active_view().clone();

                    // If the active view is `self`, this cloud-mode terminal is the root of the
                    // pane stack and has no parent terminal to host a local agent conversation.
                    if active_view.id() == self.id() {
                        log::warn!(
                            "ExitCloudModeAndStartLocalAgent received but cloud-mode pane has no parent terminal"
                        );
                    } else {
                        active_view.update(ctx, |view, ctx| {
                            view.enter_agent_view_for_new_conversation(initial_prompt, origin, ctx);
                        });
                    }
                } else {
                    log::warn!(
                        "ExitCloudModeAndStartLocalAgent received but no pane stack available; cannot start local agent without a parent terminal"
                    );
                }

                ctx.notify();
            }
            InputEvent::Escape => {
                if self.has_active_cli_agent_input_session(ctx) {
                    self.close_cli_agent_rich_input_and_disable_auto_toggle(ctx);
                    return;
                }
                if FeatureFlag::AgentView.is_enabled()
                    && self.agent_view_controller.as_ref(ctx).is_active()
                {
                    // For child agents, ESC navigates to the parent first;
                    // run this before any can-exit gating.
                    if self.try_navigate_to_parent_conversation(ctx) {
                        return;
                    }

                    // Disable escape completely for ambient agents without a parent terminal.
                    if self
                        .agent_view_controller
                        .as_ref(ctx)
                        .can_exit_agent_view()
                        .is_err()
                    {
                        return;
                    }

                    let is_long_running = self
                        .model
                        .lock()
                        .block_list()
                        .active_block()
                        .is_active_and_long_running();
                    if is_long_running && self.is_ambient_agent_session(ctx) {
                        self.exit_agent_view(ctx);
                    } else if !is_long_running {
                        // During first-time setup, always exit directly without confirmation
                        // since the setup overlay would obscure any confirmation dialog.
                        let is_in_setup = self
                            .ambient_agent_view_model
                            .as_ref()
                            .is_some_and(|model| model.as_ref(ctx).is_in_setup());
                        if !is_in_setup && !self.input.as_ref(ctx).buffer_text(ctx).is_empty() {
                            self.agent_view_controller.update(ctx, |session, ctx| {
                                session.exit_agent_view_with_required_confirmation(
                                    ExitConfirmationTrigger::Escape,
                                    ctx,
                                );
                            });
                        } else {
                            self.exit_agent_view(ctx);
                        }
                    }
                }

                // Ignore any passive blocks on escape.
                self.clear_prompt_suggestions(ctx);

                if self
                    .model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_tagged_in()
                {
                    self.tag_out_agent_for_user_long_running_command(ctx);

                    if FeatureFlag::AgentView.is_enabled()
                        && self.agent_view_controller.as_ref(ctx).is_inline()
                    {
                        self.agent_view_controller.update(ctx, |controller, ctx| {
                            controller.exit_agent_view(ctx);
                        });
                    }
                }

                ctx.emit(Event::Escape)
            }
            InputEvent::InputStateChanged(_) => {}
            InputEvent::InputEmptyStateChanged { is_empty, reason } => {
                // Update the universal developer input button bar with the new empty state
                let universal_developer_input_button_bar = self
                    .input
                    .as_ref(ctx)
                    .universal_developer_input_button_bar()
                    .clone();
                universal_developer_input_button_bar.update(ctx, |button_bar, ctx| {
                    button_bar.update_input_empty_state(*is_empty, ctx);
                });

                // When AgentView is enabled and the buffer is cleared, reset the input type
                // based on whether there's an active agent view. Skip for cloud mode v2
                // where the input is always AI.
                if FeatureFlag::AgentView.is_enabled()
                    && *is_empty
                    && !self.input.as_ref(ctx).is_cloud_mode_input_v2_composing(ctx)
                    && self
                        .ai_input_model
                        .as_ref(ctx)
                        .should_run_input_autodetection(ctx)
                {
                    let is_agent_view_active = self.agent_view_controller.as_ref(ctx).is_active();
                    let input_type = match reason {
                        InputEmptyStateChangeReason::UserCommandCompleted => InputType::Shell,
                        InputEmptyStateChangeReason::Edited => {
                            if is_agent_view_active {
                                InputType::AI
                            } else {
                                InputType::Shell
                            }
                        }
                    };

                    self.ai_input_model.update(ctx, |model, ctx| {
                        model.enable_autodetection(input_type, ctx);
                    });
                }
            }
            InputEvent::SyncInput(input) => {
                if !SyncedInputState::as_ref(ctx).is_syncing_any_inputs(ctx.window_id()) {
                    return;
                }

                match input {
                    SyncInputType::InputEditorContentsChanged { contents, .. } => {
                        ctx.emit(Event::SyncInput(SyncEvent {
                            source_view_id: self.view_id,
                            data: SyncInputType::InputEditorContentsChanged {
                                contents: contents.clone(),
                            },
                        }));
                    }
                    SyncInputType::RanCommand => {
                        ctx.emit(Event::SyncInput(SyncEvent {
                            source_view_id: self.view_id,
                            data: SyncInputType::RanCommand,
                        }));
                    }
                    // Terminal Inputs should only be sending
                    // InputEditorContentsChanged and RanCommand events.
                    _ => (),
                }
            }
            InputEvent::ShowCommandSearch(options) => {
                ctx.emit(Event::ShowCommandSearch(options.clone()));
            }
            InputEvent::CtrlD => {
                ctx.emit(Event::CtrlD);
            }
            InputEvent::CtrlC { cleared_buffer_len } => {
                self.handle_ctrl_c_input_event(*cleared_buffer_len, ctx);
            }
            InputEvent::EmacsBindingUsed => {
                if OperatingSystem::get().is_linux() && self.should_show_emacs_bindings_banner(ctx)
                {
                    self.show_emacs_bindings_banner(ctx);
                }
            }
            InputEvent::EditorUpdated {
                block_id,
                operations,
            } => {
                ctx.emit(Event::InputEditorUpdated {
                    block_id: block_id.clone(),
                    operations: operations.clone(),
                });
            }
            InputEvent::InputFocusedFromMiddleClick => {
                self.focus_input_box(ctx);
            }
            InputEvent::EditorFocused => {
                ctx.dispatch_typed_action(&PaneGroupAction::HandleFocusChange);
                ctx.notify();
            }
            InputEvent::SignupAnonymousUser { entrypoint } => {
                ctx.emit(Event::SignupAnonymousUser {
                    entrypoint: *entrypoint,
                });
            }
            InputEvent::OpenSettings(section) => {
                ctx.emit(Event::OpenSettings(*section));
            }
            #[cfg(feature = "local_fs")]
            InputEvent::OpenCodeInWarp { source, layout } => {
                ctx.emit(Event::OpenCodeInWarp {
                    source: source.clone(),
                    layout: *layout,
                });
            }
            InputEvent::OpenCodeReviewPane => {
                ctx.emit(Event::OpenCodeReviewPane(CodeReviewPanelArg {
                    repo_path: self.current_repo_path.clone(),
                    terminal_view: self.view_handle.clone(),
                    entrypoint: CodeReviewPaneEntrypoint::GitDiffChip,
                    focus_new_pane: true,
                    cli_agent: None,
                }));
            }
            InputEvent::AttachDiffSetContext {
                #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
                diff_mode,
            } => {
                #[cfg(feature = "local_fs")]
                self.handle_attach_diffset_context(diff_mode.clone(), ctx);
            }
            InputEvent::OpenConversationHistory => {
                ctx.emit(Event::OpenConversationHistory);
            }
            InputEvent::OpenProjectRulesPane => {
                self.handle_action(&TerminalAction::OpenProjectRulesPane, ctx);
            }
            InputEvent::OpenViewMCPPane => {
                self.handle_action(&TerminalAction::OpenViewMCPPane, ctx);
            }
            InputEvent::OpenAddMCPPane => {
                self.handle_action(&TerminalAction::OpenAddMCPPane, ctx);
            }
            InputEvent::OpenEnvironmentManagementPane => {
                self.open_environment_management_pane(ctx);
            }
            InputEvent::OpenFilesPalette { source } => {
                ctx.emit(Event::OpenFilesPalette { source: *source })
            }
            InputEvent::TryHandlePassiveCodeDiff(action) => {
                self.resolve_prompt_suggestion_diff(action.clone(), ctx);
            }
            InputEvent::ToggleAIDocumentPane {
                document_id,
                document_version,
            } => {
                ctx.emit(Event::ToggleAIDocumentPane {
                    document_id: *document_id,
                    document_version: *document_version,
                });
            }
            InputEvent::SubmitCLIAgentInput { text } => {
                self.submit_cli_agent_rich_input(text.clone(), ctx);
            }
            InputEvent::OpenAIDocumentPane {
                document_id,
                document_version,
            } => {
                ctx.emit(Event::OpenAIDocumentPane {
                    document_id: *document_id,
                    document_version: *document_version,
                    is_auto_open: false,
                });
            }
            InputEvent::OpenAutoReloadModal { purchased_credits } => {
                ctx.emit(Event::OpenAutoReloadModal {
                    purchased_credits: *purchased_credits,
                });
            }
            InputEvent::AuthSecretDeleteConfirmationDialogToggled { is_open } => {
                ctx.emit(Event::AuthSecretDeleteConfirmationDialogToggled { is_open: *is_open });
            }
            InputEvent::ShowToast { message, flavor } => {
                ctx.emit(Event::ShowToast {
                    message: message.clone(),
                    flavor: *flavor,
                });
            }
            InputEvent::ScrollToExchange { exchange_id } => {
                self.scroll_to_exchange(*exchange_id, ctx);
            }
            InputEvent::TriggerEnvironmentSetup { repos } => {
                self.enter_environment_setup_selector(repos.clone(), ctx);
            }
            InputEvent::RegisterPluginListener(agent) => {
                self.register_cli_agent_listener_without_session_start_event(*agent, ctx);
            }
            #[cfg(not(target_family = "wasm"))]
            InputEvent::OpenPluginInstructionsPane(agent, kind) => {
                ctx.emit(Event::OpenPluginInstructionsPane(*agent, *kind));
            }
            InputEvent::OpenShareSessionModal => {
                self.open_share_session_modal(SharedSessionActionSource::FooterChip, ctx);
            }
            InputEvent::StartRemoteControl => {
                let source = SharedSessionSource::user(
                    self.active_conversation_task_id(ctx).map(|t| t.to_string()),
                );
                self.attempt_to_share_session(
                    SharedSessionScrollbackType::All,
                    Some(SharedSessionActionSource::FooterChip),
                    source,
                    true,
                    ctx,
                );
            }
            InputEvent::OpenHandoffEnvironmentCreationModal => {
                ctx.dispatch_typed_action(&WorkspaceAction::ShowHandoffEnvironmentCreationModal);
            }
            InputEvent::OpenCloudModeV2EnvironmentCreationModal => {
                ctx.dispatch_typed_action(
                    &WorkspaceAction::ShowCloudModeV2EnvironmentCreationModal,
                );
            }
        }
    }

    pub(super) fn handle_find_event(&mut self, event: &FindEvent, ctx: &mut ViewContext<Self>) {
        match event {
            FindEvent::CloseFindBar => {
                self.close_find_bar(ctx);
                self.redetermine_global_focus(ctx);
            }
            FindEvent::Update { query } => {
                let options = self
                    .find_model
                    .as_ref(ctx)
                    .active_find_options()
                    .cloned()
                    .unwrap_or_default()
                    .with_query(query.clone());
                self.run_find(options, ctx)
            }
            FindEvent::NextMatch { direction } => self.goto_next_find_match(direction, ctx),
            FindEvent::ToggleFindInBlock { value } => self.toggle_find_within_block(ctx, *value),
            FindEvent::ToggleCaseSensitivity { is_case_sensitive } => {
                let options = self
                    .find_model
                    .as_ref(ctx)
                    .active_find_options()
                    .cloned()
                    .unwrap_or_default()
                    .with_is_case_sensitive(*is_case_sensitive);
                self.run_find(options, ctx)
            }
            FindEvent::ToggleRegexSearch { is_regex_enabled } => {
                let options = self
                    .find_model
                    .as_ref(ctx)
                    .active_find_options()
                    .cloned()
                    .unwrap_or_default()
                    .with_is_regex_enabled(*is_regex_enabled);
                self.run_find(options, ctx)
            }
        }
    }

    pub(crate) fn enter_ambient_agent_setup(
        &mut self,
        initial_prompt: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::CloudMode.is_enabled()
            || !self.model.lock().shared_session_status().is_view_pending()
        {
            // Ambient agent setup can only be done inside a shared session viewer; otherwise the backing terminal manager is incorrect.
            return;
        }

        // Don't pass an initial prompt, which auto-sends the request.
        self.enter_agent_view_for_new_conversation(None, AgentViewEntryOrigin::CloudAgent, ctx);

        if let Some(prompt) = initial_prompt {
            self.input.update(ctx, |input, ctx| {
                input.replace_buffer_content(&prompt, ctx);
            });
        }
        self.focus_input_box(ctx);
    }

    pub(super) fn last_visible_item_is_agent_view_block_for_conversation(
        &self,
        conversation_id: AIConversationId,
    ) -> bool {
        let model = self.model.lock();
        let block_list = model.block_list();

        // When we insert rich content (including agent view blocks) we insert it immediately before
        // the active block (unless explicitly inserting below a long-running block). The active
        // block is a special "warp input" block that often exists even when it isn't user-visible.
        //
        // So, for dedupe we check the first visible (non-zero height) item *immediately before the
        // active block*. This avoids false negatives caused by the active block itself.
        let active_block_index = block_list.active_block_index();

        let mut cursor = block_list
            .block_heights()
            .cursor::<BlockHeight, BlockHeightSummary>();
        cursor.descend_to_last_item(block_list.block_heights());

        // Seek backwards until we're at the active block's height item.
        while let Some(item) = cursor.item() {
            match item {
                BlockHeightItem::Block(_) if cursor.start().block_count == active_block_index.0 => {
                    break;
                }
                _ => cursor.prev(),
            }
        }

        // Now walk backwards to find the first non-hidden item before the active block.
        cursor.prev();
        while let Some(item) = cursor.item() {
            let is_hidden = item.height() == BlockHeight::zero();
            match item {
                // We use `should_hide` rather than height to determine visibility because agent view
                // entry blocks render as 0 height while agent view is active, and when we call this
                // on-agent-view-exit the sumtree hasn't been updated yet.
                BlockHeightItem::RichContent(RichContentItem {
                    view_id,
                    should_hide,
                    ..
                }) if !should_hide => {
                    if let Some(rich_content) = self
                        .rich_content_views
                        .iter()
                        .find(|content| content.view_id() == *view_id)
                    {
                        if let Some(agent_view_metadata) = rich_content.agent_view_entry_metadata()
                        {
                            if agent_view_metadata.conversation_id == conversation_id {
                                return true;
                            }
                        }
                    };
                    return false;
                }
                _ => {
                    if FeatureFlag::AgentView.is_enabled() && is_hidden {
                        cursor.prev();
                        continue;
                    } else {
                        return false;
                    }
                }
            }
        }

        false
    }

    /// Returns true when there exists an AgentViewBlock with origin LongRunningCommand that matches
    /// the given conversation id.
    pub(super) fn has_existing_lrc_agent_view_block(
        &self,
        conversation_id: AIConversationId,
    ) -> bool {
        self.rich_content_views.iter().any(|content| {
            content.agent_view_entry_metadata().is_some_and(|metadata| {
                metadata.conversation_id == conversation_id
                    && matches!(metadata.origin, AgentViewEntryOrigin::LongRunningCommand)
            })
        })
    }

    fn update_block_filter_for_block_with_active_editor(
        &mut self,
        block_filter_query: &BlockFilterQuery,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(active_filter_editor_block_index) = self.active_filter_editor_block_index else {
            log::warn!(
                "Tried to update block filter query without active_filter_editor_block_index set"
            );
            return;
        };

        let model = self.model.lock();
        let previous_filter = model.get_filter_on_block(active_filter_editor_block_index);
        if (previous_filter.is_none()
            || previous_filter
                .is_some_and(|previous_filter| !previous_filter.is_active_and_nonempty()))
            && block_filter_query.is_active_and_nonempty()
        {
            send_telemetry_from_ctx!(TelemetryEvent::UpdateBlockFilterQuery, ctx);
        }
        drop(model);

        self.update_block_filter_for_block(
            active_filter_editor_block_index,
            block_filter_query,
            ctx,
        );
    }

    /// Caches the scroll position before a filter is applied, if the filter is
    /// being applied from a zero-state. This cached scroll position is used to
    /// return users to their original scroll position when the filter is removed.
    fn maybe_cache_scroll_position_before_filter(
        &self,
        block_index: BlockIndex,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut model = self.model.lock();

        let prev_filter_query = model.get_filter_on_block(block_index);
        // Only cache the scroll position when applying a filter from a zero state.
        if !prev_filter_query.is_some_and(|query| query.is_active_and_nonempty()) {
            let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
            let viewport = self.viewport_state(model.block_list(), input_mode, ctx);
            let top_of_viewport = viewport.scroll_top_in_lines();
            let top_of_block = viewport.top_of_block_in_lines(block_index);
            let bottom_of_block = viewport.bottom_of_block_in_lines(block_index);
            // Only cache the position if the block is in the viewport.
            if height_in_range_approx(top_of_viewport, top_of_block, bottom_of_block) {
                let offset_from_block_top = top_of_viewport - top_of_block;
                model
                    .block_list_mut()
                    .set_scroll_position_before_filter(block_index, offset_from_block_top);
            }
        }
    }

    /// Set the scroll position after a filter is applied/updated. If the block
    /// is returning to a non-filtered state, we try to return the user to their
    /// original scroll position. Otherwise, we make a best effort to show the
    /// users the same lines they were seeing before a filter.
    fn update_scroll_position_after_filter(
        &mut self,
        block_index: BlockIndex,
        block_filter_query: &BlockFilterQuery,
        prev_top_of_viewport: Lines,
        prev_bottom_of_block: Lines,
        prev_first_visible_original_row: Option<usize>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !block_filter_query.is_active_and_nonempty() {
            let cached_scroll_position = self
                .model
                .lock()
                .block_list()
                .scroll_position_before_filter();
            if let Some(scroll_position) = cached_scroll_position {
                self.update_scroll_position_locking(
                    ScrollPositionUpdate::AfterFilterClear {
                        block_index: scroll_position.block_index,
                        offset_from_block_top: scroll_position.offset_from_block_top,
                    },
                    ctx,
                );
                self.model
                    .lock()
                    .block_list_mut()
                    .clear_scroll_position_before_filter();
                return;
            }
        }

        self.update_scroll_position_locking(
            ScrollPositionUpdate::AfterFilter {
                block_index,
                prev_top_of_viewport,
                prev_bottom_of_block,
                prev_first_visible_original_row,
            },
            ctx,
        );
    }

    pub(super) fn update_block_filter_for_block(
        &mut self,
        block_index: BlockIndex,
        block_filter_query: &BlockFilterQuery,
        ctx: &mut ViewContext<Self>,
    ) {
        self.maybe_cache_scroll_position_before_filter(block_index, ctx);

        let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
        // Fetch some state of the current viewport before the filter is applied.
        let (prev_top_of_viewport, prev_bottom_of_block, prev_first_visible_original_row) = {
            let model = self.model.lock();
            let viewport = self.viewport_state(model.block_list(), input_mode, ctx);
            (
                viewport.scroll_top_in_lines(),
                viewport.bottom_of_block_in_lines(block_index),
                viewport.get_first_visible_output_row(block_index),
            )
        };

        if block_filter_query.query.is_empty() {
            self.model.lock().clear_filter_on_block(block_index);
        } else {
            self.model
                .lock()
                .update_filter_on_block(block_index, block_filter_query.clone());
        };
        self.find_model.update(ctx, |find_model, ctx| {
            log::info!("Updating matches for filtered block.");
            find_model.update_matches_for_filtered_block(block_index, ctx);
        });

        let num_matched_lines = self
            .model
            .lock()
            .block_list()
            .num_matched_lines_in_filter_for_block(block_index);

        self.block_filter_editor.update(ctx, |filter_editor, ctx| {
            filter_editor.set_num_matched_lines(num_matched_lines);
            ctx.notify();
        });

        self.update_scroll_position_after_filter(
            block_index,
            block_filter_query,
            prev_top_of_viewport,
            prev_bottom_of_block,
            prev_first_visible_original_row,
            ctx,
        );

        ctx.notify();
    }

    pub(super) fn handle_block_filter_event(
        &mut self,
        event: &BlockFilterEditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            BlockFilterEditorEvent::UpdateFilter(block_filter_state) => {
                self.update_block_filter_for_block_with_active_editor(block_filter_state, ctx);
            }
            BlockFilterEditorEvent::Close => {
                self.close_block_filter_editor(ctx);
                self.redetermine_global_focus(ctx);
            }
        }
    }

    pub(super) fn handle_slow_bootstrap_banner_event(
        &mut self,
        event: &BannerEvent<TerminalAction>,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            BannerEvent::Dismiss { .. } => self.hide_slow_bootstrap_banner(ctx),
            BannerEvent::Action(terminal_action) => {
                self.handle_action(terminal_action, ctx);
            }
        }
    }

    pub(super) fn handle_incompatible_configuration_banner_event(
        &mut self,
        event: &BannerEvent<TerminalAction>,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            BannerEvent::Dismiss { .. } => {
                self.is_incompatible_configuration_banner_open = false;
                ctx.notify();
            }
            BannerEvent::Action(_) => {
                #[cfg(debug_assertions)]
                log::warn!("Incomptabile configuration banner does not support handling actions");
            }
        }
    }

    /// Whether the incompatible shell configuration banner is open.
    pub fn is_incompatible_configuration_banner_open(&self) -> bool {
        self.is_incompatible_configuration_banner_open
    }

    pub(super) fn handle_emacs_bindings_banner_clicked(
        &mut self,
        event: &BannerEvent<TerminalAction>,
        ctx: &mut ViewContext<Self>,
    ) {
        if matches!(event, BannerEvent::Dismiss(DismissalType::Temporary)) {
            set_custom_keybinding(SELECT_ALL_BINDING_NAME, &CTRL_SHIFT_A_KEYSTROKE, ctx);
            set_custom_keybinding(MOVE_LINE_START_BINDING_NAME, &CTRL_A_KEYSTROKE, ctx);
            set_custom_keybinding(MOVE_LINE_END_BINDING_NAME, &CTRL_E_KEYSTROKE, ctx);
        }
        EmacsBindingsSettings::handle(ctx).update(ctx, |settings_model, settings_ctx| {
            report_if_error!(settings_model
                .emacs_bindings_banner_state
                .set_value(BannerState::Dismissed, settings_ctx));
        });
        self.is_emacs_bindings_banner_open = false;
        ctx.notify();
    }

    fn should_show_emacs_bindings_banner(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        // Is this the active session?
        // We should only show the banner in one place at a time.
        if !self.is_active_session(ctx) {
            return false;
        }

        // Was the banner already open or dismissed?
        let emacs_bindings_banner_displayed = self.is_emacs_bindings_banner_open
            || EmacsBindingsSettings::handle(ctx).read(ctx, |banner_settings, _| {
                *banner_settings.emacs_bindings_banner_state.value() == BannerState::Dismissed
            });

        !emacs_bindings_banner_displayed
    }

    fn show_emacs_bindings_banner(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_emacs_bindings_banner_open = true;
        ctx.notify();
    }

    /// Updates the state of the "incompatible shell configuration" banner with
    /// a new set of shell plugins. This should be called when either a new session
    /// is bootstrapped or the `honor_ps1` setting changes.
    pub(super) fn update_incompatible_configuration_banner(
        &mut self,
        shell_plugins: &HashSet<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let honor_ps1 = *SessionSettings::as_ref(ctx).honor_ps1;

        let show_banner = if honor_ps1 {
            let banner_content = if shell_plugins.contains("p10k_unsupported") {
                Some(BannerTextContent::formatted_text(vec![
                    FormattedTextFragment::bold("Powerlevel10k now supports Warp!  "),
                    FormattedTextFragment::plain_text(
                        "You seem to be running an older (unsupported) version, please follow ",
                    ),
                    FormattedTextFragment::hyperlink(
                        "these instructions",
                        P10K_UPDATE_INSTRUCTIONS_URL,
                    ),
                    FormattedTextFragment::plain_text(" to update to the latest version."),
                ]))
            } else if shell_plugins.contains("pure") {
                Some(BannerTextContent::formatted_text(vec![
                    FormattedTextFragment::plain_text(
                        "Pure is not yet supported in Warp. You might consider one of the \
                        supported prompts as an alternative.  ",
                    ),
                    FormattedTextFragment::hyperlink("Learn more", PROMPT_COMPATIBILITY_URL),
                ]))
            } else {
                None
            };

            if let Some(banner_content) = banner_content {
                self.incompatible_configuration_banner
                    .update(ctx, |banner, ctx| {
                        banner.set_content(banner_content, ctx);
                    });
                true
            } else {
                false
            }
        } else {
            false
        };

        if show_banner != self.is_incompatible_configuration_banner_open {
            self.is_incompatible_configuration_banner_open = show_banner;
            ctx.notify();
        }
    }

    pub(super) fn handle_controlmaster_error_banner_event(
        &mut self,
        event: &BannerEvent<TerminalAction>,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            BannerEvent::Dismiss { .. } => {
                self.control_master_error_banner_state.is_open = false;
                ctx.notify();
            }
            BannerEvent::Action(_) => {
                #[cfg(debug_assertions)]
                unimplemented!(
                    "Control master error banner does not yet support handling terminal actions"
                );
            }
        }
    }

    pub(super) fn open_block_list_context_menu_via_keybinding(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(block_index) = self.selected_blocks.tail() {
            // We are manually putting the selected block in the hover state
            // before we open the context menu since
            // 1. The buttons need to be visible when the context menu is open
            // 2. We need to use the saved position of the overflow button
            // to know where to open up the context menu, which is only saved
            // using the position ID that includes the block index when a block
            // is hovered. Otherwise, we will have a panic.
            self.hovered_block_index = Some(block_index);
            self.scroll_to_if_not_visible(block_index, ctx);
            self.block_list_context_menu(
                &BlockListMenuSource::BlockKeybinding { block_index },
                ctx,
            );
            ctx.notify();
        }
    }
}
