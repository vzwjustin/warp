use super::*;

impl TerminalView {
    pub(super) fn user_write_ctrl_c_to_pty(&mut self, ctx: &mut ViewContext<Self>) {
        self.write_user_bytes_to_pty(vec![escape_sequences::C0::ETX], ctx);
    }

    pub(super) fn handle_ctrl_c_input_event(
        &mut self,
        cleared_buffer_len: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        let did_resolve_prompt_suggestion = self
            .resolve_passive_suggestion(PromptSuggestionResolution::Reject { ctrl_c: true }, ctx);
        if did_resolve_prompt_suggestion {
            if FeatureFlag::AgentView.is_enabled()
                && self.agent_view_controller.as_ref(ctx).is_active()
            {
                self.agent_view_controller.update(ctx, |controller, ctx| {
                    controller.clear_pending_exit_confirmation(ctx);
                });
            }
            return;
        }

        if FeatureFlag::AgentView.is_enabled() && self.agent_view_controller.as_ref(ctx).is_active()
        {
            if cleared_buffer_len > 0 {
                self.agent_view_controller.update(ctx, |controller, ctx| {
                    controller.clear_pending_exit_confirmation(ctx);
                });
                return;
            }

            if self.should_ctrl_c_exit_agent_view(ctx) {
                self.agent_view_controller.update(ctx, |controller, ctx| {
                    controller.exit_agent_view_with_required_confirmation(
                        ExitConfirmationTrigger::CtrlC,
                        ctx,
                    );
                });
                return;
            }
        }

        self.ctrl_c(ctx);
    }

    /// Windows users expect ctrl-c to copy if there is selected text. Otherwise,
    /// we perform the normal ctrl-c action.
    pub(super) fn ctrl_c(&mut self, ctx: &mut ViewContext<Self>) {
        let (
            has_block_list_selection,
            has_alt_screen_selection,
            is_long_running,
            is_agent_in_control_of_command,
        ) = {
            let model = self.model.lock();
            let has_alt_screen_selection = model.alt_screen().selection().is_some();
            let has_block_list_selection = model.block_list().selection().is_some();
            let active_block = model.block_list().active_block();
            let is_long_running = active_block.is_active_and_long_running();
            let is_agent_in_control_of_command = active_block.is_agent_in_control();
            (
                has_block_list_selection,
                has_alt_screen_selection,
                is_long_running,
                is_agent_in_control_of_command,
            )
        };
        // We don't want to copy blocks in AI input mode because those are
        // context blocks.
        let has_copiable_block_selection = !self.selected_blocks.is_empty()
            && !self.ai_input_model.as_ref(ctx).is_ai_input_enabled();

        self.ctrl_c_internal(
            has_copiable_block_selection,
            has_block_list_selection,
            has_alt_screen_selection,
            is_long_running,
            is_agent_in_control_of_command,
            ctx,
        );

        // We want to focus the input/rich content block if it is active.
        self.redetermine_global_focus(ctx);
        ctx.notify();
    }

    /// Copy if there is a selection. Otherwise, we defer to the normal ctrl-c
    /// behaviour.

    /// Focuses the provided AI block if this terminal view (or some part of it)
    /// are focused. This helps ensure AI block interactions (which are primarily async)
    /// don't steal focus from the user if they've focused another part of the app
    /// (e.g. another session).
    ///
    /// Warning: this should not be called when focusing the [`TerminalView`]. It could
    /// lead to a focus cycle because [`AIBlock::try_focus`] conditionally yields focus
    /// back to the [`TerminalView`].
    pub(super) fn focus_ai_block_if_self_focused(
        &self,
        block: &ViewHandle<AIBlock>,
        ctx: &mut ViewContext<Self>,
    ) {
        if ctx.is_self_or_child_focused() {
            block.update(ctx, |block, ctx| block.try_steal_focus(ctx));
        }
    }

    pub(super) fn ctrl_c_to_active_block(
        &mut self,
        is_long_running: bool,
        is_agent_in_control_of_command: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if is_agent_in_control_of_command {
            self.cli_subagent_controller.update(ctx, |controller, ctx| {
                controller.switch_control_to_user(UserTakeOverReason::Stop, ctx);
            });
        } else if is_long_running {
            self.user_write_ctrl_c_to_pty(ctx);
        } else {
            self.maybe_handle_ctrl_c_in_rich_content_block(ctx);
        }
    }

    /// Returns whether ctrl-c should exit the agent view.
    ///
    /// This is true when:
    /// - Agent view feature is enabled
    /// - Agent view is active and can be exited
    /// - No long-running command
    /// - Conversation is not in progress and not blocked
    fn should_ctrl_c_exit_agent_view(&self, app: &AppContext) -> bool {
        if !FeatureFlag::AgentView.is_enabled() {
            return false;
        }

        if !self.agent_view_controller.as_ref(app).is_active() {
            return false;
        }

        if self
            .agent_view_controller
            .as_ref(app)
            .can_exit_agent_view()
            .is_err()
        {
            return false;
        }

        // Cannot use ctrl-c to exit agent view if there's a long-running command.
        let model = self.model.lock();
        if model
            .block_list()
            .active_block()
            .is_active_and_long_running()
        {
            return false;
        }

        let history_model = BlocklistAIHistoryModel::as_ref(app);
        if let Some(conversation) = history_model.active_conversation(self.view_id) {
            let is_new_empty_conversation = self
                .agent_view_controller
                .as_ref(app)
                .agent_view_state()
                .is_new()
                && conversation.is_empty();
            let status = conversation.status();
            // Additionally check if the conversation is empty, since the default status for a new
            // conversation is `InProgress`, but you should be able to exit an empty conversation.
            if (status.is_in_progress() || status.is_blocked()) && !is_new_empty_conversation {
                return false;
            }
        }

        true
    }

    /// Cancels the active agent conversation via the status bar's Ctrl+C handler.
    /// Includes shared session notification if applicable.
    fn cancel_active_conversation_via_status_bar(&mut self, ctx: &mut ViewContext<Self>) {
        if FeatureFlag::AgentSharedSessions.is_enabled()
            && self
                .model
                .lock()
                .shared_session_status()
                .is_sharer_or_viewer()
        {
            self.input.update(ctx, |input, ctx| {
                input.cancel_active_agent_conversation_for_shared_session(
                    CancellationReason::ManuallyCancelled,
                    ctx,
                );
            });
        }

        let status_bar = self.input.as_ref(ctx).agent_status_bar().clone();
        status_bar.update(ctx, |status_bar, ctx| {
            status_bar.handle_ctrl_c(ctx);
        });
    }

    /// If there is an active rich content block that is set up to handle ctrl-c
    /// events, allow it to handle the event.
    ///
    /// TODO(CORE-3415): We should probably remove the FixedBindings for ctrl-c
    /// in the SSH warpification blocks and handle them here as well.
    fn maybe_handle_ctrl_c_in_rich_content_block(&mut self, ctx: &mut ViewContext<Self>) {
        if self.active_ai_block(ctx).is_some() {
            self.cancel_active_conversation_via_status_bar(ctx);
        } else if BlocklistAIHistoryModel::as_ref(ctx)
            .active_conversation(self.view_id)
            .is_some_and(|c| c.status().is_in_progress())
        {
            // No unfinished AI block, but the conversation is still in progress.
            // This happens when a server-side subagent (e.g., conversation search)
            // is running — the parent AI block is already finished but the response
            // stream is still active. Route Ctrl+C to the status bar to cancel it.
            self.cancel_active_conversation_via_status_bar(ctx);
        } else if self.has_active_init_project(ctx) {
            if let Some(model) = &self.active_init_project_model {
                model.update(ctx, |m, ctx| m.cancel(ctx));
            }
        } else if let Some(active_init_env_block) = self.active_init_environment_block(ctx) {
            active_init_env_block.update(ctx, |init_env_block, ctx| {
                init_env_block.handle_ctrl_c(ctx);
            });
        } else if self
            .passive_suggestions_models
            .legacy
            .as_ref(ctx)
            .is_passive_code_diff_being_generated()
        {
            // Handle Ctrl-C for passive code generation blocks ("Generating fix..." state)
            self.abort_prompt_and_code_suggestions(ctx);
        } else if let Some(active_env_var_block) = self.active_env_var_collection_block(ctx) {
            active_env_var_block.update(ctx, |env_var_block, ctx| {
                env_var_block.handle_ctrl_c(ctx);
            });
        }
    }

    pub(super) fn ctrl_d(&mut self, ctx: &mut ViewContext<Self>) {
        let arc = self.model.clone();
        let mut model = arc.lock();

        // Only write EOT to the PTY if the input box is not visible, which would
        // happen iff there is a long-running block. The one exception is when
        // the PTY is still bootstrapping, in which case the input would be shown
        // but we still want EOT written to the PTY in case there is a program
        // waiting for input during bootstrapping (e.g. omz update).
        if !self.is_input_box_visible(&model, ctx) || !model.block_list().is_bootstrapped() {
            // This is relevant for the case where the user enters CTRL-d while the
            // ssh wrapper command is being run. The EOT character doesn't stop
            // the session immediately, instead it waits until the command passed
            // to it is complete. This means that the SSH wrapper command will
            // still send the InitShell message to the terminal. In order to
            // prevent it from being processed, we keep state and clear it on
            // the next precmd (i.e. when the command completes).
            model.ignore_bootstrapping_messages();

            // Drop the model before writing bytes to the pty, otherwise we
            // get a deadlock.  This model locking is a bit of a mess.
            drop(model);
            self.write_user_bytes_to_pty(&[escape_sequences::C0::EOT][..], ctx);
        }
    }

    pub fn is_long_running(&self) -> bool {
        let model = self.model.lock();
        model
            .block_list()
            .active_block()
            .is_active_and_long_running()
            && !model.is_read_only()
    }

    /// Returns `true` when an interactive SSH command has been detected at
    /// preexec and the SSH block is still running (long-running). Used by
    /// the workspace to derive `PendingRemoteSession` without storing
    /// mutable state on the workspace itself.
    pub fn has_pending_ssh_command(&self) -> bool {
        self.warpify_state.get_pending_ssh_host().is_some() && self.is_long_running()
    }

    /// Like `is_long_running`, but also requires the user to be in control of the command
    /// (i.e. the user ran it, or took it over from the agent). Returns `false` for commands
    /// that are currently being driven by the agent.
    pub fn is_long_running_and_user_controlled(&self) -> bool {
        let model = self.model.lock();
        let active_block = model.block_list().active_block();
        active_block.is_active_and_long_running()
            && !active_block.is_agent_driving_command()
            && !model.is_read_only()
    }

    pub fn was_ever_visible(&self) -> bool {
        self.was_ever_visible
    }

    pub fn content_element_height_lines(&self, app: &AppContext) -> Lines {
        element_size_at_last_frame(&self.content_element_position_id, self.window_id, app)
            .map(|size| Pixels::new(size.y()))
            .unwrap_or(self.size_info.pane_height_px())
            .to_lines(self.size_info.cell_height_px)
    }

    pub fn content_element_height_px(&self, app: &AppContext) -> f32 {
        element_size_at_last_frame(&self.content_element_position_id, self.window_id, app)
            .map(|size| size.y())
            .unwrap_or(self.size_info.pane_height_px().as_f32())
    }

    pub fn content_element_width_px(&self, app: &AppContext) -> f32 {
        element_size_at_last_frame(&self.content_element_position_id, self.window_id, app)
            .map(|size| size.x())
            .unwrap_or(self.size_info.pane_height_px().as_f32())
    }

    pub(super) fn user_input_sequence(&mut self, code: &[u8], ctx: &mut ViewContext<Self>) {
        let sequence = EscCodes::build_escape_sequence(self.model.lock().deref(), code);
        self.control_sequence_on_terminal(&sequence, ctx);
    }

    pub(super) fn control_sequence_on_terminal(
        &mut self,
        bytes: &[u8],
        ctx: &mut ViewContext<Self>,
    ) {
        if self.is_long_running() {
            self.on_ssh_warpification_key_event(Some(SshKeyEvent::from_bytes(bytes)), ctx);
            self.write_user_bytes_to_pty(bytes.to_owned(), ctx);
        } else {
            safe_warn!(
                safe: ("command not long-running. ignoring control seq on terminal."),
                full: ("command not long-running. ignoring control seq on terminal: {:?}", bytes)
            )
        }
    }

    /// Emits an event indicating that this session has an active alt-screen or
    /// long-running and received keyboard input.
    /// Emits if at least one terminal inputs is synced, so receivers of this
    /// event must determine how to process this event.
    /// Also emits an event for shared session viewers to notify the sharer of a write to pty request.
    fn emit_non_editor_typed_event(&self, chars: Vec<u8>, ctx: &mut ViewContext<Self>) {
        if SyncedInputState::as_ref(ctx).is_syncing_any_inputs(ctx.window_id()) {
            ctx.emit(Event::SyncInput(SyncEvent {
                source_view_id: self.view_id,
                data: SyncInputType::NonEditorTyped {
                    chars: Arc::new(chars.clone()),
                },
            }));
        }

        self.model
            .lock()
            .send_write_to_pty_events_for_shared_session(chars);
    }

    pub(super) fn update_scroll_position_locking(
        &mut self,
        update: ScrollPositionUpdate,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut model = self.model.lock();
        // Clear the cached pre-filter scroll position if a non-filter user
        // event is detected.
        if !matches!(
            update,
            ScrollPositionUpdate::AfterFilter { .. } | ScrollPositionUpdate::AfterResize
        ) {
            model.block_list_mut().clear_scroll_position_before_filter();
        }
        let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
        let viewport = self.viewport_state(model.block_list(), input_mode, ctx);
        if self.scroll_position.update(viewport, update, ctx) {
            ctx.notify();
            // Dismiss any visible tooltips when the scroll position changes
            drop(model);
            self.dismiss_tooltips(ctx);
        }
    }

    pub fn set_show_pane_accent_border(
        &mut self,
        show_accent_border: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.pane_configuration.update(ctx, |pane_config, ctx| {
            pane_config.set_show_accent_border(show_accent_border, ctx);
            ctx.notify();
        });
    }

    /// Receiving the warpui::Event::KeyDown event from a child element.
    /// Generally, this should be control characters rather than printable characters.
    pub(super) fn keydown_on_terminal(&mut self, characters: &str, ctx: &mut ViewContext<Self>) {
        if self.is_long_running() {
            self.on_ssh_warpification_key_event(Some(SshKeyEvent::from_chars(characters)), ctx);
            self.highlighted_link.invalidate();
            self.report_possible_typeahead(characters);
            self.write_user_bytes_to_pty(characters.as_bytes().to_vec(), ctx);
        } else {
            // When it's not a long-running command, we want to clear the selected block
            // and focus the editor. We specifically don't want to insert
            // anything into the input box. Characters that belong there should go through
            // `typed_characters_on_terminal` rather than through here.
            self.clear_selected_blocks(ctx);
            self.clear_selected_text(ctx);

            self.update_scroll_position_locking(ScrollPositionUpdate::AfterKeydownOnTerminal, ctx);
            self.redetermine_global_focus(ctx);
        }
    }

    fn should_write_typed_chars_to_pty(&self, ctx: &mut ViewContext<Self>) -> bool {
        // Lock the model once and hold it throughout the function
        let model = self.model.lock();

        // If the active block hasn't started yet, we don't want to write to the pty.
        // Note that we check block started and NOT block.is_long_running(), because
        // the block starts on enter but only becomes long running on receiving Preexec.
        // We want to make sure we capture any input between enter and receiving Preexec.
        if !model.block_list().active_block().started() {
            return false;
        }

        // Make sure we don't write any text to the pty until we've echoed out
        // the bootstrap script, otherwise the user could accidentally interfere
        // with bootstrap script execution.
        let was_bootstrap_script_echoed = self
            .sessions
            .as_ref(ctx)
            .has_pending_or_bootstrapped_session();
        let is_shared_session_executor = model.shared_session_status().is_executor();

        was_bootstrap_script_echoed || is_shared_session_executor
    }
    /// Receiving a warpui::Event::TypedCharacters event from a child element.
    /// We can assume `characters` consists of all printable characters, and therefore,
    /// can go into the input box.
    pub(super) fn typed_characters_on_terminal(
        &mut self,
        characters: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        self.on_ssh_warpification_key_event(Some(SshKeyEvent::from_chars(characters)), ctx);

        if self.should_write_typed_chars_to_pty(ctx) {
            self.highlighted_link.invalidate();
            self.report_possible_typeahead(characters);
            self.write_user_bytes_to_pty(characters.as_bytes().to_vec(), ctx);
        } else {
            // We should only insert typed characters into the input box buffer.
            // When input_sequence is triggered on KeyDown, we should focus
            // on the input area and let the editor view handle the TypedCharacters
            // event. When it is triggered on TypedCharacters, we should pass
            // the received string down to input view.

            // Only clear selected blocks and text if we're not in AI mode since in AI mode we
            // don't want to clear the selected blocks or text (context) when we start typing.
            //
            // When `FeatureFlag::AgentView` is enabled, blocks are attachable as AI context in
            // terminal mode. Selections are preserved so they can be attached to the query when
            // entering the agent view.
            if !self.ai_render_context.borrow().is_ai_input_enabled
                && !FeatureFlag::AgentView.is_enabled()
            {
                self.clear_selected_blocks(ctx);
                self.clear_selected_text(ctx);
            }

            self.update_scroll_position_locking(ScrollPositionUpdate::AfterTypedCharacters, ctx);
            self.input
                .update(ctx, |input, ctx| input.system_insert(characters, ctx));
        }
    }

    /// Handles a file-tree drag-and-drop onto the terminal by piping the dropped text
    /// through the same path as user-typed characters for the active command.
    pub fn handle_file_tree_drop_on_active_command(
        &mut self,
        text: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        self.typed_characters_on_terminal(text, ctx);
    }

    pub(super) fn set_marked_text_on_terminal(
        &mut self,
        marked_text: &str,
        selected_range: &Range<usize>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !FeatureFlag::ImeMarkedText.is_enabled() {
            return;
        }
        self.model
            .lock()
            .set_marked_text(marked_text, selected_range);
        ctx.notify();
    }

    pub(super) fn clear_marked_text_on_terminal(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::ImeMarkedText.is_enabled() {
            return;
        }
        self.model.lock().clear_marked_text();
        ctx.notify();
    }

    pub(crate) fn write_to_pty<B: Into<Cow<'static, [u8]>>>(
        &mut self,
        data: B,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(Event::WriteBytesToPty { bytes: data.into() });
    }

    pub(super) fn write_agent_bytes_to_pty<B: Into<Cow<'static, [u8]>>>(
        &mut self,
        data: B,
        mode: &AIAgentPtyWriteMode,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(Event::WriteAgentInputToPty {
            bytes: data.into(),
            mode: *mode,
        });
    }

    /// Writes a shared session viewer's bytes to the pty
    pub fn write_viewer_bytes_to_pty(&mut self, bytes: Vec<u8>, ctx: &mut ViewContext<Self>) {
        self.write_user_bytes_to_pty(bytes, ctx);
    }

    /// Ends the current line before writing 1000 byte chunks to the pty with a small delay in
    /// between to work around a macos pty bug.
    pub(super) fn clear_line_editor_and_write_to_pty_with_mac_workaround_hack<
        B: Into<Cow<'static, [u8]>>,
    >(
        &mut self,
        data: B,
        ctx: &mut ViewContext<Self>,
    ) {
        // Ctrl-u + ctrl-k clears everything before the cursor, then everything after the cursor.
        // Ctrl-c is dangerous because it could cancel an ongoing command. We add an arbitrary space
        // first so that the ctrl-u always clears at least one character, avoiding the audible bell.
        let mut to_write = vec![
            b' ',
            escape_sequences::C0::VT,  // ctrl-k to clear forward
            escape_sequences::C0::NAK, // ctrl-u to clear backward
        ];
        to_write.extend_from_slice(&data.into());

        for (i, chunk) in to_write.chunks(1000).enumerate() {
            let chunk = chunk.to_vec();
            ctx.spawn(
                Timer::after(Duration::from_millis(i as u64 * 10)),
                move |me, _, ctx| me.write_to_pty(chunk, ctx),
            );
        }
    }

    /// Ends the current line before writing the given bytes to the PTY.
    pub(super) fn clear_line_editor_and_write_to_pty<B: Into<Cow<'static, [u8]>>>(
        &mut self,
        data: B,
        ctx: &mut ViewContext<Self>,
    ) {
        // Ctrl-u + ctrl-k clears everything before the cursor, then everything after the cursor.
        // Ctrl-c is dangerous because it could cancel an ongoing command. We add an arbitrary space
        // first so that the ctrl-u always clears at least one character, avoiding the audible bell.
        let mut to_write = vec![
            b' ',
            escape_sequences::C0::VT,  // ctrl-k to clear forward
            escape_sequences::C0::NAK, // ctrl-u to clear backward
        ];
        to_write.extend_from_slice(&data.into());
        self.write_to_pty(to_write, ctx);
    }

    /// Writes to the PTY, resets selected blocks and updates scroll position.
    /// Also calls logic to emit a sync event.
    pub(super) fn write_user_bytes_to_pty<B: Into<Cow<'static, [u8]>>>(
        &mut self,
        data: B,
        ctx: &mut ViewContext<Self>,
    ) {
        {
            let mut terminal_model = self.model.lock();
            let active_block = terminal_model.block_list().active_block();
            if active_block.is_agent_in_control() {
                return;
            }
            if active_block.is_active_and_long_running() && !active_block.has_received_user_input()
            {
                terminal_model
                    .block_list_mut()
                    .active_block_mut()
                    .mark_received_user_input();
            }
        }

        let bytes = data.into();
        let bytes_vec = bytes.to_vec();
        self.clear_selected_blocks(ctx);
        self.update_scroll_position_locking(ScrollPositionUpdate::AfterWriteUserBytesToPty, ctx);
        self.write_to_pty(bytes, ctx);
        self.emit_non_editor_typed_event(bytes_vec, ctx);
    }

    /// Write to the PTY if the session has finished bootstrapping and
    /// has an active long-running command.
    /// Never emits a sync event.
    pub(super) fn write_to_pty_for_syncing_long_running_commands(
        &mut self,
        characters: Vec<u8>,
        ctx: &mut ViewContext<Self>,
    ) {
        let was_bootstrap_script_echoed = self
            .sessions
            .as_ref(ctx)
            .has_pending_or_bootstrapped_session();
        // Make sure we don't write any text to the pty until we've echoed out
        // the bootstrap script, otherwise the user could accidentally interfere
        // with bootstrap script execution.
        if was_bootstrap_script_echoed && self.is_long_running() {
            self.clear_selected_blocks(ctx);
            self.update_scroll_position_locking(
                ScrollPositionUpdate::AfterWriteUserBytesToPty,
                ctx,
            );
            self.write_to_pty(characters, ctx);
        }
    }

    /// Report user input to the terminal's typeahead model as potential typeahead.
    /// The model matches the input recorded here against the actual characters
    /// echoed to the pty to determine what is typeahead.
    fn report_possible_typeahead(&mut self, input: &str) {
        self.model.lock().push_user_input(input);
    }

    pub fn set_pending_command(&self, exec: &str, ctx: &mut ViewContext<Self>) {
        self.input.update(ctx, |input, ctx| {
            input.set_pending_command(exec, ctx);
        })
    }

    pub fn set_pending_command_queue(
        &mut self,
        commands: Vec<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.pending_command_queue = commands.into_iter().collect();
        self.set_next_pending_command_from_queue(ctx);
    }

    pub(super) fn set_next_pending_command_from_queue(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if self.input.as_ref(ctx).has_pending_command() {
            return false;
        }
        let Some(command) = self.pending_command_queue.pop_front() else {
            return false;
        };

        self.set_pending_command(&command, ctx);
        true
    }

    pub(super) fn alt_scroll_cmd_sequence(&self, lines_to_scroll: i32) -> Vec<u8> {
        let cmd = if lines_to_scroll > 0 {
            EscCodes::ARROW_UP
        } else {
            EscCodes::ARROW_DOWN
        };
        EscCodes::build_escape_sequence_with_c1(C1::SS3, &[cmd])
    }

    pub(super) fn alt_scroll_sequences(&mut self, lines_to_scroll: i32) -> Vec<u8> {
        let cmd = self.alt_scroll_cmd_sequence(lines_to_scroll);
        let lines = lines_to_scroll.unsigned_abs();
        let mut content = Vec::with_capacity(lines as usize * 3);

        for _ in 0..lines {
            content.extend_from_slice(&cmd);
        }
        content
    }

    pub(super) fn alt_scroll(&mut self, lines_to_scroll: i32, ctx: &mut ViewContext<Self>) {
        // Scrolling on the alt screen can cause the grid content to change, so any link highlights are
        // no longer valid.
        self.highlighted_link.invalidate();

        let content = self.alt_scroll_sequences(lines_to_scroll);
        self.write_user_bytes_to_pty(content, ctx);
        ctx.notify();
    }

    pub fn input_size_at_last_frame(&self, app: &AppContext) -> Option<Vector2F> {
        app.element_position_by_id_at_last_frame(self.window_id, &self.input_position_id)
            .map(|bounds| bounds.size())
    }

    pub fn viewport_state<'a>(
        &self,
        block_list: &'a BlockList,
        input_mode: InputMode,
        app: &AppContext,
    ) -> ViewportState<'a> {
        let content_element_size =
            element_size_at_last_frame(&self.content_element_position_id, self.window_id, app)
                .unwrap_or(self.size_info.pane_size_px());
        ViewportState::new(
            block_list,
            self.snackbar_header_state.clone(),
            input_mode,
            *self.size_info,
            self.scroll_position.position(),
            None,
            self.horizontal_clipped_scroll_state.clone(),
            content_element_size,
            self.input_size_at_last_frame(app).unwrap_or_default(),
            if BlocklistAIHistoryModel::as_ref(app)
                .active_conversation(self.view_id)
                .is_some()
            {
                AutoscrollBehavior::WhenScrolledToEnd
            } else {
                AutoscrollBehavior::Always
            },
            self.inline_menu_positioner.clone(),
        )
    }

    /// Dismisses any open tooltips on the grid, returning whether any were actually closed.
    pub fn dismiss_tooltips(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let was_open = self.is_any_tooltip_open();
        self.open_grid_link_tool_tip = None;
        self.open_secret_tool_tip = None;
        self.open_rich_content_link_tool_tip = None;
        for rich_content in self.rich_content_views.iter() {
            if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                ai_metadata.ai_block_handle.update(ctx, |ai_block, ctx| {
                    ai_block.dismiss_ai_tooltips(ctx);
                });
            }
        }
        if was_open {
            ctx.notify();
            // The mouse cursor may have been over the tooltip before it was dismissed. Reset it to
            // clear any lingering alternate pointers.
            ctx.reset_cursor();
        }
        was_open
    }

    pub(super) fn is_any_tooltip_open(&self) -> bool {
        self.open_grid_link_tool_tip.is_some()
            || self.open_secret_tool_tip.is_some()
            || self.open_rich_content_link_tool_tip.is_some()
    }

    pub(super) fn handle_sessions_event(
        &mut self,
        event: SessionsEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SessionsEvent::SessionInitialized { .. } => {
                self.handle_session_initialized(ctx);
            }
            SessionsEvent::SessionBootstrapped(event) => {
                self.handle_session_bootstrapped(*event, ctx);
            }
            _ => {}
        }
    }

    pub(super) fn scroll(&mut self, delta: Lines, ctx: &mut ViewContext<Self>) {
        self.dismiss_tooltips(ctx);
        self.update_scroll_position_locking(
            ScrollPositionUpdate::AfterScrollEvent {
                scroll_delta: delta,
            },
            ctx,
        );
        ctx.notify();
    }

    pub(super) fn handle_typeahead_event(&mut self, ctx: &mut ViewContext<Self>) {
        let mut model = self.model.lock();
        let completed_block_idx = model.block_list().prev_matching_block_from_index(
            BlockFilter {
                include_hidden: true,
                include_background: false,
            },
            model.block_list().active_block_index(),
        );
        let was_typeahead_entered_during_ai_requested_command =
            completed_block_idx.is_some_and(|idx| {
                model
                    .block_list()
                    .block_at(idx)
                    .is_some_and(|block| block.agent_interaction_metadata().is_some())
            });

        let Some((typeahead, num_typeahead_chars_inserted)) = model
            .block_list_mut()
            .early_output_mut()
            .advance_typeahead()
        else {
            #[cfg(feature = "integration_tests")]
            log::warn!("Received typeahead event, but typeahead was empty");

            return;
        };

        // We don't insert typeahead into the input buffer when it was entered during an
        // agent-requested command - the agent is going to follow-up immediately after the
        // command exists anyway, not to mention the expected semantics of typeahead are
        // probably different with AI requested commands because the input remains interactive
        // (for at least the first few seconds of the command's execution).
        if !was_typeahead_entered_during_ai_requested_command {
            #[cfg(feature = "integration_tests")]
            log::info!("Writing typeahead to input editor: {typeahead}");

            self.input.update(ctx, |input, ctx| {
                input.insert_typeahead_text(num_typeahead_chars_inserted, typeahead, ctx);
            });
            ctx.notify();
        }
    }

    /// This function is invoked every time there is some form of view event
    /// such as a state change or terminal wakeup to update the view context.
    pub(super) fn handle_terminal_wakeup(&mut self, _: (), ctx: &mut ViewContext<Self>) {
        // If find bar is active, we update the matches for the last/active block or the alt screen.
        if self.find_model.as_ref(ctx).is_find_bar_open() {
            self.find_model.update(ctx, |find_model, ctx| {
                find_model.rerun_find_on_active_grid(ctx);
            });
        }

        // For simplicity, we simply rescan the entire block for block filter matches.
        self.model
            .lock()
            .block_list_mut()
            .maybe_refilter_active_block_output();

        // If the block filter editor is open on an active block we update the
        // number of line matches.
        if let Some(block_index) = self.active_filter_editor_block_index {
            let model = self.model.lock();
            let active_block_index = model.block_list().active_block_index();
            let num_matched_lines = model
                .block_list()
                .num_matched_lines_in_filter_for_block(block_index);
            if block_index == active_block_index {
                self.block_filter_editor.update(ctx, |filter_editor, ctx| {
                    filter_editor.set_num_matched_lines(num_matched_lines);
                    ctx.notify();
                });
            }
        }

        // The active block height could have changed since the last time it was calculated, as
        // one cause of the Wakeup signal is the long-running process timer. Make sure that the
        // model is up-to-date with the current height information.
        if !self.model.lock().is_alt_screen_active() {
            let mut model = self.model.lock();
            model.block_list_mut().update_background_block_height();
            model.block_list_mut().update_active_block_height();
        }
        self.maybe_emit_terminal_view_state_changed_for_long_running_block(ctx);
        self.use_agent_footer.update(ctx, |footer, ctx| {
            footer.notify_and_notify_children(ctx);
        });

        // Need to re-render both the alt screen and the blocklist on keypresses.
        ctx.notify();
    }

    /// This function is invoked whenever we detect an SSH ControlMaster error,
    /// in which case completions will not work as expected.
    pub(super) fn handle_control_master_error(&mut self, ctx: &mut ViewContext<Self>) {
        let active_session_id = self.active_block_session_id();
        // We don't want to display the error banner a second time in a given session
        // if the user has already closed it.  When we open the banner initially, we
        // store the session ID in here, so if the stored value matches the current
        // session, we've already shown the banner.
        //
        // TODO(vorporeal): This logic falls apart for nested ssh sessions - we could
        // show the banner in the outer ssh session, show it again for the inner ssh
        // session, then forgot that we already showed it for the outer session.  This
        // probably won't happen often, but it's something that we might want to clean
        // up eventually.
        if self.control_master_error_banner_state.associated_session_id != active_session_id {
            let has_remote_server = active_session_id.is_some_and(|session_id| {
                self.sessions
                    .as_ref(ctx)
                    .get(session_id)
                    .is_some_and(|session| {
                        matches!(
                            session.session_type(),
                            SessionType::WarpifiedRemote {
                                host_id: Some(_),
                                ..
                            }
                        )
                    })
            });

            // Don't show the banner when the session already has a remote server
            // active — the CTA to enable the SSH extension is irrelevant.
            self.control_master_error_banner_state = ControlMasterErrorBannerState {
                is_open: !has_remote_server,
                associated_session_id: active_session_id,
            };

            ctx.notify();

            send_telemetry_from_ctx!(
                TelemetryEvent::SSHControlMasterError { has_remote_server },
                ctx
            );
        }
    }

    pub(super) fn read_from_clipboard(
        shell_family: Option<ShellFamily>,
        ctx: &mut ViewContext<Self>,
    ) -> String {
        let content = ctx.clipboard().read();
        clipboard_content_with_escaped_paths(content, shell_family, false)
    }

    pub(super) fn middle_click_paste_content(
        shell_family: Option<ShellFamily>,
        ctx: &mut ViewContext<Self>,
    ) -> String {
        let content = SelectionSettings::handle(ctx).update(ctx, |selection, ctx| {
            selection.read_for_middle_click_paste(ctx)
        });

        content
            .map(|content| clipboard_content_with_escaped_paths(content, shell_family, false))
            .unwrap_or_default()
    }

    /// Turns the active session into a bootstrapped subshell by writing the InitShell DCS hook
    pub(super) fn trigger_subshell_bootstrap(
        &mut self,
        shell_type: Option<ShellType>,
        triggered_by_rc_file_snippet: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.dismiss_warpify_banner(&RememberForWarpification::DoNotRememberSubshellCommand, ctx);

        // Record the active long-running block so we can hide it later once the remote
        // actually confirms subshell bootstrap is in progress.
        // If the remote never emits InitShell, the block stays visible.
        {
            let model = self.model.lock();
            if model
                .block_list()
                .active_block()
                .is_active_and_long_running()
            {
                let block_id = model.block_list().active_block_id().clone();
                self.warpify_state.set_block_id(block_id);
            }
        }

        self.write_init_subshell_bytes_to_pty(shell_type, ctx);

        if !self.env_vars.is_empty() {
            self.start_bootstrap_timer(ENV_VAR_BOOTSTRAP_FAILED_DURATION, ctx);
            self.env_vars = Vec::new();
        } else {
            self.start_bootstrap_timer(BOOTSTRAP_FAILED_DURATION, ctx);
        }

        send_telemetry_from_ctx!(
            TelemetryEvent::TriggerSubshellBootstrap {
                triggered_by_rc_file_snippet
            },
            ctx
        );
    }

    /// Util method to update the ssh block, with a lock
    pub(super) fn update_long_running_ssh_block_with_lock(
        &self,
        f: impl FnOnce(&mut Block),
    ) -> bool {
        if let Some(block_id) = self.warpify_state.block_id() {
            if let Some(block) = self
                .model
                .lock()
                .block_list_mut()
                .mut_block_from_id(&block_id)
            {
                f(block);
                return true;
            }
        }
        false
    }

    pub(super) fn cancel_bootstrap_workflow(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_ssh_blocks(ctx);
        self.update_long_running_ssh_block_with_lock(|block| {
            block.unhide();
        });
        self.warpify_state.delete_state();
        ctx.notify();
    }

    pub(super) fn remove_ssh_block_by_id(&mut self, view_id: EntityId) {
        self.model
            .lock()
            .block_list_mut()
            .remove_rich_content(view_id);
    }

    pub(super) fn clear_ssh_blocks(&mut self, ctx: &mut ViewContext<Self>) {
        self.dismiss_warpify_banner(&RememberForWarpification::DoNotRememberSSHHost, ctx);
        if let Some(ssh_block) = self.warpify_state.ssh_block_state() {
            let view_id = ssh_block.get_block_view_id();

            self.remove_ssh_block_by_id(view_id);

            self.redetermine_global_focus(ctx);

            self.warpify_state.clear_ssh_block_state();
        }
    }
}
