use super::*;

impl TerminalView {
    pub(super) fn handle_windowing_state_update(
        &mut self,
        (current, previous): (&windowing::State, &windowing::State),
        ctx: &mut ViewContext<Self>,
    ) {
        let window_changed = previous.active_window != current.active_window;
        let is_active_window_current_window = Some(ctx.window_id()) == current.active_window;

        if window_changed {
            if let Some(focus_out_window_id) = previous.active_window {
                if focus_out_window_id == ctx.window_id() && ctx.is_self_or_child_focused() {
                    self.maybe_report_focus_out(ctx);
                }
            }

            if let Some(focus_in_window_id) = current.active_window {
                if focus_in_window_id == ctx.window_id() && ctx.is_self_or_child_focused() {
                    self.maybe_report_focus_in(ctx);
                }
            }
        }

        // When we change windows, we need to update the timestamp of the newly focused terminal view.
        if window_changed && is_active_window_current_window && ctx.is_self_or_child_focused() {
            self.last_focus_ts = Some(chrono::Local::now().naive_local());
        }
    }

    pub fn sessions_model(&self) -> &ModelHandle<Sessions> {
        &self.sessions
    }

    /// Returns `None` for local sessions, `Some("user@hostname")` for remote.
    /// Used to key per-host plugin install failure tracking.
    pub(super) fn active_session_remote_host<C: ModelAsRef>(&self, ctx: &C) -> Option<String> {
        self.active_block_session_id().and_then(|session_id| {
            let session = self.sessions.as_ref(ctx).get(session_id)?;
            if session.is_local() {
                None
            } else {
                Some(format!("{}@{}", session.user(), session.hostname()))
            }
        })
    }

    /// Returns whether a specific session is local, treating shared-session
    /// viewers and conversation transcript viewers as non-local even when
    /// their session hasn't been joined yet.
    pub fn session_is_local<C: ModelAsRef>(&self, session_id: SessionId, ctx: &C) -> bool {
        let forced_non_local = {
            let model = self.model.lock();
            model.is_shared_session_viewer() || model.is_conversation_transcript_viewer()
        };
        !forced_non_local
            && self
                .sessions
                .as_ref(ctx)
                .get(session_id)
                .is_some_and(|session| session.is_local())
    }

    /// Returns whether or not the active session is a local session.  Returns
    /// None if there is no active session.
    pub fn active_session_is_local<C: ModelAsRef>(&self, ctx: &C) -> Option<bool> {
        Some(self.session_is_local(self.active_block_session_id()?, ctx))
    }

    /// Returns the active session's launch shell, if it is specified.
    /// Returns None if there is no active session or if the current session does not
    /// have a launch shell.
    pub fn active_session_shell<C: ModelAsRef>(&self, ctx: &C) -> Option<ShellLaunchData> {
        self.active_block_session_id().and_then(|session_id| {
            let current_session = self.sessions.as_ref(ctx).get(session_id)?;
            current_session.launch_data().cloned()
        })
    }

    /// Returns the active session's WSL distribution information, if it exists.
    /// Returns None if there is no active session or if the current session is
    /// not a WSL session.
    pub fn active_session_wsl_distro<C: ModelAsRef>(&self, ctx: &C) -> Option<String> {
        self.active_block_session_id().and_then(|session_id| {
            let current_session = self.sessions.as_ref(ctx).get(session_id)?;
            let distro_name = current_session.wsl_distro_name();
            distro_name.map(|name| name.to_string())
        })
    }

    pub fn active_block_session_id(&self) -> Option<SessionId> {
        self.active_block_metadata
            .as_ref()
            .and_then(BlockMetadata::session_id)
    }

    pub fn active_session_shell_type<C: ModelAsRef>(&self, ctx: &C) -> Option<ShellType> {
        self.active_block_session_id()
            .and_then(|id| self.sessions.as_ref(ctx).get(id))
            .map(|s| s.shell().shell_type())
    }

    pub fn active_session_path_if_local<C: ModelAsRef>(&self, ctx: &C) -> Option<PathBuf> {
        if self.active_session_is_local(ctx) == Some(true) {
            self.active_block_metadata
                .as_ref()
                .and_then(BlockMetadata::current_working_directory)
                .and_then(|cwd| {
                    self.active_block_session_id()
                        .and_then(|active_session_id| {
                            self.sessions.as_ref(ctx).get(active_session_id)
                        })
                        .and_then(|active_session| {
                            active_session
                                .launch_data()
                                .and_then(|data| data.maybe_convert_absolute_path(cwd))
                        })
                })
                // Checking if the pwd from the active session actually exists
                // and if not (ie. directory was removed) - return None.
                .filter(|path| path.is_dir())
        } else {
            None
        }
    }

    pub fn input(&self) -> &ViewHandle<Input> {
        &self.input
    }

    pub fn input_config(&self, app: &AppContext) -> InputConfig {
        self.ai_input_model.as_ref(app).input_config()
    }

    /// Applies an input mode update from an external source (e.g., session sharing).
    /// This bypasses normal event emission to prevent update loops.
    pub fn apply_external_input_mode_update(
        &mut self,
        config: InputConfig,
        ctx: &mut ViewContext<Self>,
    ) {
        self.input.update(ctx, |input, ctx| {
            input.apply_external_input_config_update(config, ctx);
        });
    }

    pub fn ai_controller(&self) -> &ModelHandle<BlocklistAIController> {
        &self.ai_controller
    }

    pub fn ai_context_model(&self) -> &ModelHandle<BlocklistAIContextModel> {
        &self.ai_context_model
    }

    pub fn ai_input_model(&self) -> &ModelHandle<BlocklistAIInputModel> {
        &self.ai_input_model
    }

    pub fn agent_view_controller(&self) -> &ModelHandle<AgentViewController> {
        &self.agent_view_controller
    }

    pub fn active_conversation_id(&self, app: &AppContext) -> Option<AIConversationId> {
        self.agent_view_controller
            .as_ref(app)
            .agent_view_state()
            .active_conversation_id()
    }

    pub fn active_conversation_task_id(&self, app: &AppContext) -> Option<AmbientAgentTaskId> {
        let history = BlocklistAIHistoryModel::as_ref(app);
        let conversation_id = self.active_conversation_id(app).or_else(|| {
            self.ai_context_model
                .as_ref(app)
                .selected_conversation_id(app)
        })?;
        history.conversation(&conversation_id)?.task_id()
    }

    pub fn ambient_agent_view_model(
        &self,
    ) -> Option<&ModelHandle<ambient_agent::AmbientAgentViewModel>> {
        self.ambient_agent_view_model.as_ref()
    }

    pub(super) fn ambient_agent_task_id_for_details_panel_from_model(
        &self,
        model: &TerminalModel,
        app: &AppContext,
    ) -> Option<AmbientAgentTaskId> {
        self.ambient_agent_view_model
            .as_ref()
            .and_then(|model| model.as_ref(app).task_id())
            .or_else(|| model.ambient_agent_task_id())
    }
    pub fn ambient_agent_task_id_for_details_panel(
        &self,
        app: &AppContext,
    ) -> Option<AmbientAgentTaskId> {
        let model = self.model.lock();
        self.ambient_agent_task_id_for_details_panel_from_model(&model, app)
    }

    /// Whether the conversation details side panel should be available in the
    /// pane header / pane layout for this terminal view.
    pub(super) fn can_show_conversation_details_ui_from_model(
        &self,
        model: &TerminalModel,
        app: &AppContext,
    ) -> bool {
        self.ambient_agent_task_id_for_details_panel_from_model(model, app)
            .is_some()
            || BlocklistAIHistoryModel::as_ref(app)
                .active_conversation(self.view_id)
                .is_some_and(|conversation| !conversation.is_empty())
    }

    /// Convenience wrapper around
    /// [`Self::can_show_conversation_details_ui_from_model`] that locks the
    /// terminal model. Do not call from contexts that already hold the lock.
    pub(super) fn can_show_conversation_details_ui(&self, app: &AppContext) -> bool {
        let model = self.model.lock();
        self.can_show_conversation_details_ui_from_model(&model, app)
    }

    /// Consume the one-shot conversation details panel auto-open for this
    /// view. Call this before the first `maybe_auto_open_conversation_details_panel`
    /// fires (e.g. on a parent-orchestrated child agent pane) so the panel does
    /// not default open. Manual toggle via `TerminalAction::ToggleConversationDetailsPanel`
    /// continues to work normally.
    pub(crate) fn suppress_initial_conversation_details_panel_auto_open(&mut self) {
        self.conversation_details_panel_auto_open_policy =
            ConversationDetailsPanelAutoOpenPolicy::DefaultClosed;
    }

    pub(super) fn maybe_insert_tombstone_for_non_running_shared_ambient_task(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::CloudModeSetupV2.is_enabled() {
            return;
        }

        let (task_id, is_active_shared_session, is_finished_viewer) = {
            let model = self.model.lock();
            if model.is_receiving_agent_conversation_replay() {
                return;
            }

            let status = model.shared_session_status();
            // This method also handles restored cloud-mode panes that rendered
            // a conservative tombstone before task data arrived. When the task
            // cache updates, either the existing tombstone or FinishedViewer
            // status tells us to re-resolve the CTA/input state.
            let should_update = model.is_shared_ambient_agent_session()
                || self.conversation_ended_tombstone_view_id.is_some()
                || status.is_finished_viewer();
            if !should_update {
                return;
            }

            (
                self.ambient_agent_task_id_for_details_panel_from_model(&model, ctx),
                status.is_active_viewer() || status.is_active_sharer(),
                status.is_finished_viewer(),
            )
        };

        let Some(task_id) = task_id else {
            return;
        };
        let Some(task) = AgentConversationsModel::as_ref(ctx).get_task_data(&task_id) else {
            return;
        };

        if !task.is_no_longer_running() || self.pending_cloud_followup_task_id.is_some() {
            return;
        }

        if FeatureFlag::HandoffCloudCloud.is_enabled() {
            if is_active_shared_session {
                return;
            }
            let Some(state) = self.cloud_conversation_continuation_ui_state(ctx) else {
                return;
            };
            match state {
                CloudConversationContinuationUiState::Tombstone { cta } => {
                    self.insert_conversation_ended_tombstone_with_cta(cta, ctx);
                }
                CloudConversationContinuationUiState::FollowupInput => {
                    if self.conversation_ended_tombstone_view_id.is_some() || is_finished_viewer {
                        self.insert_conversation_ended_tombstone_with_resolved_cta(ctx);
                    } else {
                        self.enable_cloud_followup_input(task_id, ctx);
                    }
                }
            }
        } else {
            self.insert_conversation_ended_tombstone_with_cta(None, ctx);
        }
    }

    pub fn active_session(&self) -> &ModelHandle<ActiveSession> {
        &self.active_session
    }

    pub fn find_bar(&self) -> &ViewHandle<Find<TerminalFindModel>> {
        &self.find_bar
    }

    pub fn has_highlighted_link(&self) -> bool {
        self.highlighted_link.is_some()
    }

    pub fn hovered_block_index(&self) -> Option<BlockIndex> {
        self.hovered_block_index
    }

    pub fn is_context_menu_open(&self) -> bool {
        self.context_menu_state.is_some()
    }

    pub fn last_focus_ts(&self) -> Option<NaiveDateTime> {
        self.last_focus_ts
    }

    pub fn is_read_only(&self) -> bool {
        self.model.lock().is_read_only()
    }

    /// Whether this terminal pane is responsible for uploading a file.
    pub fn is_ssh_uploader(&self) -> bool {
        self.is_ssh_file_uploader
    }

    pub fn set_is_ssh_uploader(&mut self, is_uploader: bool) {
        self.is_ssh_file_uploader = is_uploader;
    }

    /// Whether or not this terminal view is actively sharing its session.
    pub fn is_sharing_session(&self) -> bool {
        self.model.lock().shared_session_status().is_active_sharer()
    }

    pub fn is_shared_ambient_agent_session(&self) -> bool {
        self.model.lock().is_shared_ambient_agent_session()
    }

    pub fn is_shared_session_viewer(&self) -> bool {
        self.model.lock().is_shared_session_viewer()
    }

    pub(crate) fn apply_viewer_shared_session_input_update(
        &mut self,
        block_id: &BlockId,
        operations: Vec<CrdtOperation>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.should_suppress_ambient_setup_input_sync(ctx) {
            return;
        }

        self.input().update(ctx, |input, ctx| {
            input.process_remote_edits(block_id, operations, ctx);
        });
    }

    pub(super) fn should_suppress_ambient_setup_input_sync(&self, app: &AppContext) -> bool {
        FeatureFlag::CloudModeSetupV2.is_enabled()
            && self.ambient_agent_view_model.as_ref().is_some_and(|model| {
                let model = model.as_ref(app);
                let setup_state = model.setup_command_state();
                setup_state.should_suppress_input_sync_for_current_group()
            })
    }

    pub fn ssh_file_upload(&self) -> &ViewHandle<FileUpload> {
        &self.ssh_file_upload
    }

    pub(super) fn should_report_focus(&self, ctx: &mut ViewContext<Self>) -> bool {
        let model = self.model.lock();
        let focus_reporting_enabled = *AltScreenReporting::as_ref(ctx)
            .focus_reporting_enabled
            .value();
        focus_reporting_enabled
            && model.is_alt_screen_active()
            && model.alt_screen().is_mode_set(TermMode::FOCUS_IN_OUT)
    }

    pub(super) fn maybe_report_focus_in(&mut self, ctx: &mut ViewContext<Self>) {
        if self.should_report_focus(ctx) && !self.is_focused_and_active {
            self.write_to_pty(EscCodes::FOCUS_IN, ctx);
        }
        self.is_focused_and_active = true;
    }

    pub(super) fn contains_restored_remote_blocks(&self) -> bool {
        !self
            .model
            .lock()
            .block_list()
            .blocks()
            .iter()
            .all(|block| block.restored_block_was_local().unwrap_or(true))
    }

    // This logic is only needed if the user has disabled AI in remote sessions.
    // It has potential performance implications if called on every focus change,
    // so we limit it to only when the user disables AI in remote sessions.
    pub(super) fn update_focused_terminal_info(&mut self, ctx: &mut ViewContext<Self>) {
        if !ctx.is_self_or_child_focused() {
            return;
        }

        let is_ai_allowed_in_remote_sessions =
            UserWorkspaces::as_ref(ctx).is_ai_allowed_in_remote_sessions();

        // Only update the FocusedTerminalInfo model if the user has disabled AI in remote sessions
        // because it's a potentially expensive operation.
        if !is_ai_allowed_in_remote_sessions {
            let contains_remote_blocks = self.any_session_contains_remote_blocks;
            let contains_restored_remote_blocks = self.any_session_contains_restored_remote_blocks;
            let updated = FocusedTerminalInfo::handle(ctx).update(
                ctx,
                |model: &mut FocusedTerminalInfo, ctx| {
                    model.update(contains_remote_blocks, contains_restored_remote_blocks, ctx)
                },
            );
            if updated {
                ctx.notify();
            }
        }
    }

    pub(super) fn maybe_report_focus_out(&mut self, ctx: &mut ViewContext<Self>) {
        if self.should_report_focus(ctx) && self.is_focused_and_active {
            self.write_to_pty(EscCodes::FOCUS_OUT, ctx);
        }
        self.is_focused_and_active = false;
    }

    /// Returns the `EntityId` of this view.
    pub fn id(&self) -> EntityId {
        self.view_id
    }

    pub fn pane_configuration(&self) -> &ModelHandle<PaneConfiguration> {
        &self.pane_configuration
    }

    pub fn is_input_box_visible(&self, model: &TerminalModel, app: &AppContext) -> bool {
        if model.is_read_only() {
            return false;
        }
        if self.conversation_ended_tombstone_view_id.is_some() {
            return false;
        }
        if self.has_active_cli_agent_input_session(app) {
            return true;
        }
        if model.is_alt_screen_active()
            && !model.block_list().active_block().is_agent_in_control()
            && !model.block_list().active_block().is_agent_tagged_in()
        {
            return false;
        }

        if model.shared_session_status().is_view_pending() && !self.is_ambient_agent_session(app) {
            return false;
        }

        // In cloud agent conversations, once the shared session is ready but before the first
        // agent exchange arrives, we hide the interactive input view. A non-interactive footer is
        // rendered instead (see `TerminalView::render`).
        if !FeatureFlag::CloudModeSetupV2.is_enabled()
            && !FeatureFlag::HandoffCloudCloud.is_enabled()
            && ambient_agent::is_cloud_agent_pre_first_exchange(
                self.ambient_agent_view_model.as_ref(),
                &self.agent_view_controller,
                model,
                app,
            )
        {
            return false;
        }

        if self.has_active_init_project(app) && self.is_last_block_init_step(app) {
            return false;
        }

        if FeatureFlag::CreateEnvironmentSlashCommand.is_enabled()
            && self.active_init_environment_block(app).is_some()
        {
            return false;
        }

        if self.active_env_var_collection_block(app).is_some() {
            return false;
        }

        // Hide the input box while the SSH remote-server choice block is shown.
        // User must choose to install or skip before any shell input is possible.
        if self.active_ssh_remote_server_choice_block().is_some() {
            return false;
        }

        // Hide the input box during the entire remote-server setup flow.
        // The loading footer renders instead.
        if FeatureFlag::SshRemoteServer.is_enabled() {
            if let Some(pending_sid) = model.pending_session_id() {
                if self
                    .sessions
                    .as_ref(app)
                    .remote_server_setup_state(pending_sid)
                    .is_some_and(|state| state.is_in_progress())
                {
                    return false;
                }
            }
        }

        let active_ai_block = self.active_ai_block(app);
        if active_ai_block.is_some_and(|ai_block| {
            let ai_block = ai_block.as_ref(app);
            ai_block.is_blocked_on_user_confirmation(app)
                || ai_block.has_expanded_running_commands(app)
        }) {
            return false;
        }

        let active_command_block = model.block_list().active_block();
        let is_active_and_long_running = active_command_block.is_active_and_long_running();
        let is_oz_env_startup_command = active_command_block.is_oz_environment_startup_command();
        let is_running_in_band_command =
            model.block_list().is_writing_or_executing_in_band_command();

        let has_active_long_running_agent_interaction =
            active_command_block.is_agent_monitoring() || active_command_block.is_agent_tagged_in();

        if (active_ai_block.is_none() || has_active_long_running_agent_interaction)
            && is_active_and_long_running
            && (!FeatureFlag::CloudModeSetupV2.is_enabled() || !is_oz_env_startup_command)
            && !is_running_in_band_command
            && model.block_list().is_bootstrapped()
        {
            // Show the input if:
            // * The agent is control of the active, long running block, so long as the agent is not blocked.
            // * OR the user has 'tagged in' the agent.
            return (active_command_block.is_agent_in_control()
                && !active_command_block.is_agent_blocked())
                || active_command_block.is_agent_tagged_in();
        }

        true
    }

    pub(super) fn should_render_legacy_ambient_agent_loading_footer(
        &self,
        model: &TerminalModel,
        app: &AppContext,
    ) -> bool {
        !model.is_read_only()
            && !FeatureFlag::CloudModeSetupV2.is_enabled()
            && !FeatureFlag::HandoffCloudCloud.is_enabled()
            && ambient_agent::is_cloud_agent_pre_first_exchange(
                self.ambient_agent_view_model.as_ref(),
                &self.agent_view_controller,
                model,
                app,
            )
    }

    /// Give the agent control of the active long running command
    /// (which was started outside of a conversation).
    pub(super) fn tag_agent_in(&mut self, ctx: &mut ViewContext<Self>) {
        self.model
            .lock()
            .block_list_mut()
            .active_block_mut()
            .set_is_agent_tagged_in(true);

        if !self.model.lock().is_alt_screen_active() {
            self.hide_use_agent_footer_in_blocklist(ctx);
        }

        self.input.update(ctx, |input, ctx| {
            input.set_input_mode_agent(true, ctx);
            input.clear_buffer_and_reset_undo_stack(ctx);
        });
        ctx.notify();
    }

    // Take control back from the agent for the active long running command
    // (which was started outside of a conversation).
    pub(super) fn tag_agent_out(&mut self, ctx: &mut ViewContext<Self>) {
        if !self
            .model
            .lock()
            .block_list()
            .active_block()
            .is_agent_tagged_in()
        {
            return;
        }

        self.model
            .lock()
            .block_list_mut()
            .active_block_mut()
            .set_is_agent_tagged_in(false);

        if !self.model.lock().is_alt_screen_active() {
            self.maybe_show_use_agent_footer_in_blocklist(ctx);
        }

        self.input.update(ctx, |input, ctx| {
            input.set_input_mode_terminal(false, ctx);
        });
        self.redetermine_terminal_focus(ctx);

        ctx.notify();
    }

    pub(super) fn emit_long_running_command_agent_interaction_state_changed(
        &self,
        agent_has_control: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let state = if agent_has_control {
            LongRunningCommandAgentInteractionState::InControl
        } else {
            let is_tagged_in = self
                .model
                .lock()
                .block_list()
                .active_block()
                .is_agent_tagged_in();
            if is_tagged_in {
                LongRunningCommandAgentInteractionState::TaggedIn
            } else {
                LongRunningCommandAgentInteractionState::NotInteracting
            }
        };
        log::info!(
            "emit_long_running_command_agent_interaction_state_changed: \
             agent_has_control={agent_has_control}, emitting state={state:?}"
        );
        ctx.emit(Event::LongRunningCommandAgentInteractionStateChanged { state });
    }

    /// Applies a long-running command agent interaction state received from a shared session participant.
    pub fn apply_long_running_command_agent_interaction_state(
        &mut self,
        state: LongRunningCommandAgentInteractionState,
        ctx: &mut ViewContext<Self>,
    ) {
        match state {
            LongRunningCommandAgentInteractionState::InControl => {
                self.cli_subagent_controller.update(ctx, |controller, ctx| {
                    controller.handoff_active_command_control_to_agent(ctx);
                });
            }
            LongRunningCommandAgentInteractionState::TaggedIn => {
                self.cli_subagent_controller.update(ctx, |controller, ctx| {
                    controller.switch_control_to_user(UserTakeOverReason::Manual, ctx);
                });
                self.tag_agent_in(ctx);
            }
            LongRunningCommandAgentInteractionState::NotInteracting => {
                self.cli_subagent_controller.update(ctx, |controller, ctx| {
                    controller.switch_control_to_user(UserTakeOverReason::Manual, ctx);
                });
                self.tag_agent_out(ctx);
            }
        }
    }

    /// Shows or hides the CLI agent footer from a shared session update.
    pub fn apply_cli_agent_footer_visibility(&mut self, show: bool, ctx: &mut ViewContext<Self>) {
        if show {
            self.maybe_show_use_agent_footer_in_blocklist(ctx);
        } else {
            self.hide_use_agent_footer_in_blocklist(ctx);
        }
    }

    pub fn has_active_env_var_block(&self, app: &AppContext) -> bool {
        self.active_env_var_collection_block(app).is_some()
    }

    /// Shuts down the pty and event loop, terminating the shell process.
    /// Also marks this view as manually shut down for telemetry attribution.
    pub fn shutdown_pty(&mut self, ctx: &mut ViewContext<Self>) {
        self.manual_pty_shutdown_requested = true;
        ctx.emit(Event::ShutdownPty);
    }

    pub(crate) fn stop_local_agent_conversation(
        &mut self,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let had_active_stream = self
            .ai_controller
            .as_ref(ctx)
            .has_active_stream_for_conversation(conversation_id, ctx);

        self.ai_controller.update(ctx, |controller, ctx| {
            controller.cancel_conversation_progress(
                conversation_id,
                CancellationReason::ManuallyCancelled,
                ctx,
            );
        });

        let visible_conversation_id = self
            .agent_view_controller
            .as_ref(ctx)
            .agent_view_state()
            .active_conversation_id();
        let history_active_conversation_id =
            BlocklistAIHistoryModel::as_ref(ctx).active_conversation_id(self.view_id);

        let should_interrupt_active_command = {
            let mut model = self.model.lock();
            let active_block = model.block_list_mut().active_block_mut();
            let active_block_matches = active_block.ai_conversation_id() == Some(conversation_id)
                || visible_conversation_id == Some(conversation_id)
                || history_active_conversation_id == Some(conversation_id);
            let command_is_running = active_block.is_executing()
                || active_block.is_command_grid_active()
                || active_block.is_active_and_long_running();

            if active_block_matches && command_is_running {
                active_block.set_user_control_with_stop_reason();
                true
            } else {
                false
            }
        };

        if should_interrupt_active_command {
            self.user_write_ctrl_c_to_pty(ctx);
        }

        if !had_active_stream {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.update_conversation_status(
                    self.view_id,
                    conversation_id,
                    ConversationStatus::Cancelled,
                    ctx,
                );
            });
        }
    }
}
