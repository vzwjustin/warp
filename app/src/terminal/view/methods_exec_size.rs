use super::*;

impl TerminalView {
    pub fn execute_pending_command(&mut self, _: (), ctx: &mut ViewContext<Self>) {
        let had_pending = self.input.read(ctx, |input, _| input.has_pending_command());
        self.input.update(ctx, |input, ctx| {
            input.execute_pending_command(ctx);
        });
        // If the pending command was just consumed, track that we're waiting
        // for the resulting block to complete.
        if had_pending && !self.input.read(ctx, |input, _| input.has_pending_command()) {
            self.awaiting_pending_command_completion = true;
        }
    }

    // Try to execute the provided command. If we cannot execute it now, set it as the pending
    // command.
    //
    // If we set it as pending, the command will execute when we trigger another call to
    // `execute_pending_command` (either from a `BlockCompleted` or `BootstrapPrecmdDone` event)
    pub fn execute_command_or_set_pending(&mut self, command: &str, ctx: &mut ViewContext<Self>) {
        self.set_pending_command(command, ctx);
        self.execute_pending_command((), ctx);
    }

    pub(super) fn hide_slow_bootstrap_banner(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_slow_bootstrap_banner_open {
            self.is_slow_bootstrap_banner_open = false;
            ctx.notify();
        }
    }

    pub fn is_login_shell_bootstrapped(&self) -> bool {
        self.is_login_shell_bootstrapped
    }
    pub fn has_pending_command_or_awaiting_completion(&self, ctx: &AppContext) -> bool {
        self.awaiting_pending_command_completion
            || !self.pending_command_queue.is_empty()
            || self.input.as_ref(ctx).has_pending_command()
    }

    /// Marks this terminal to enter agent view once pending setup commands
    /// finish. Called from `pane_tree_from_template_recursive` when the tab
    /// config has both commands and `PaneMode::Agent`.
    pub fn set_enter_agent_view_after_pending_commands(&mut self) {
        self.enter_agent_view_after_pending_commands = true;
    }

    /// Clears the deferred agent view entry flag. Called by the workspace
    /// during onboarding to keep the session in terminal mode for the
    /// guided tutorial.
    pub fn clear_enter_agent_view_after_pending_commands(&mut self) {
        self.enter_agent_view_after_pending_commands = false;
    }

    /// Start a timer so that we can detect when a session does not bootstrap in a timely manner
    pub(super) fn start_bootstrap_timer(&self, duration: Duration, ctx: &mut ViewContext<Self>) {
        let _ = ctx.spawn(
            async move {
                warpui::r#async::Timer::after(duration).await;
            },
            Self::on_bootstrap_failed_timer_complete,
        );
    }

    /// Called once the bootstrap timer completes
    ///
    /// Will send telemetry if the current session is not bootstrapped and will show a banner to
    /// the user if this is the first bootstrap in the session.
    fn on_bootstrap_failed_timer_complete(&mut self, _: (), ctx: &mut ViewContext<Self>) {
        let (is_ssh, shell, is_subshell, was_triggered_by_rc_file, is_wsl, is_msys2) = {
            let model = self.model.lock();

            // If we did actually bootstrap, or if the session is no longer usable
            // (e.g.: the shell process terminated), don't show a banner.
            if model.is_read_only() || model.is_active_block_bootstrapped() {
                return;
            }

            let is_ssh = model.has_pending_ssh_session();
            let shell = model
                .pending_shell_type()
                .map_or("unknown", |shell| shell.name());
            let pending_subshell_info = model.pending_subshell_session();
            let is_subshell = pending_subshell_info.is_some();
            let was_triggered_by_rc_file = pending_subshell_info
                .map(|info| info.was_triggered_by_rc_file_snippet)
                .unwrap_or(false);
            let is_wsl = model.is_pending_wsl();
            let is_msys2 = model.is_pending_msys2();

            (
                is_ssh,
                shell,
                is_subshell,
                was_triggered_by_rc_file,
                is_wsl,
                is_msys2,
            )
        };

        log::warn!("Bootstrapping failed for shell {shell:?} on ssh {is_ssh}");

        // Unhide the long-running block that was hidden at the start of
        // subshell bootstrap so the user can see the session output again.
        self.update_long_running_ssh_block_with_lock(|block| {
            block.unhide();
        });

        // Send the bootstrapping slow event synchronously to ensure that we don't drop
        // the event if the user quits the app before the event queue is flushed and then
        // never reopens the app.
        send_telemetry_sync_from_ctx!(
            TelemetryEvent::BootstrappingSlow(BootstrappingInfo {
                shell,
                is_ssh,
                is_subshell,
                is_wsl,
                is_msys2,
                was_triggered_by_rc_file,
                bootstrap_duration_seconds: None,
                shell_version: None,
                rcfiles_duration_seconds: None,
                warp_attributed_bootstrap_duration_seconds: None,
                terminal_session_id: None,
            }),
            ctx
        );

        let bootstrap_block_contents = {
            let model = self.model.lock();
            model.block_list().bootstrap_block_contents()
        };
        send_telemetry_sync_from_ctx!(
            TelemetryEvent::BootstrappingSlowContents(SlowBootstrapInfo {
                shell,
                is_ssh,
                is_subshell,
                is_wsl,
                is_msys2,
                bootstrap_block_contents,
            }),
            ctx
        );

        if !self.is_login_shell_bootstrapped {
            log::warn!("Showing bootstrap slow toast");
            self.is_slow_bootstrap_banner_open = true;
            ctx.notify();
        }

        ctx.emit(Event::SlowBootstrap);
    }

    pub fn size_info(&self) -> &SizeInfo {
        &self.size_info
    }

    pub fn colors(&self) -> &color::List {
        &self.colors
    }

    pub fn override_colors(&self) -> color::OverrideList {
        let override_colors = self.model.lock().override_colors();
        override_colors
    }

    pub(super) fn appearance<'a>(&self, ctx: &'a ViewContext<Self>) -> &'a Appearance {
        Appearance::as_ref(ctx)
    }

    pub(super) fn refresh_size(&mut self, ctx: &mut ViewContext<Self>) {
        self.resize_internal(
            SizeUpdateBuilder::for_refresh(*self.size_info).build(self, ctx),
            ctx,
        )
    }

    pub(super) fn resize_internal(&mut self, size_update: SizeUpdate, ctx: &mut ViewContext<Self>) {
        // Viewer-driven sizing: report the viewer's natural size to the sharer.
        // This runs before the early-return so the initial report on viewer join
        // fires even when the pane size hasn't changed yet.
        // The resize-reason check prevents loops (SharerSizeChanged is never re-reported).
        self.maybe_report_viewer_terminal_size(&size_update, ctx);

        // If this isn't an actionable resize, there's nothing to do.
        if !(size_update.anything_changed() || size_update.is_refresh()) {
            return;
        }

        let new_size = size_update.new_size.pane_size_px();
        if new_size.x() == 0. || new_size.y() == 0. {
            log::info!("Tried to resize with size {new_size:?}. Skipping resize");
            return;
        }

        // Update model with new size info.
        self.model.lock().resize(size_update);
        self.find_model.update(ctx, |find_model, ctx| {
            find_model.rerun_find_on_active_grid(ctx);
        });
        // Resizing the model already clears selected text, but
        // we also need to clear selections in any rich content blocks (e.g. AI blocks).
        if size_update.rows_or_columns_changed() {
            self.clear_selected_text(ctx);
        }

        // Update view data with new size info.
        self.input.update(ctx, |view, ctx| {
            view.set_size_info(size_update.new_size, ctx);
            view.notify_and_notify_children(ctx);
        });
        self.inline_menu_positioner.update(ctx, |positioner, ctx| {
            positioner.set_size_info(size_update.new_size, ctx);
        });
        *self.size_info = size_update.new_size;
        self.update_scroll_position_locking(ScrollPositionUpdate::AfterResize, ctx);

        // Notify subscribers.
        ctx.emit(Event::Resize { size_update });
    }

    /// If we're a viewer eligible for viewer-driven sizing, report our natural
    /// terminal size to the sharer — but only when the resize was NOT caused by
    /// the sharer (which would create a loop).
    fn maybe_report_viewer_terminal_size(
        &mut self,
        size_update: &SizeUpdate,
        ctx: &mut ViewContext<Self>,
    ) {
        if size_update.is_sharer_size_change() {
            return;
        }
        if !self.model.lock().shared_session_status().is_active_viewer() {
            return;
        }
        let eligible = self.is_viewer_driven_sizing_eligible(false, ctx);
        if eligible {
            let new_natural = (size_update.natural_rows(), size_update.natural_cols());
            let last_reported = self
                .shared_session_viewer()
                .and_then(|v| v.last_reported_natural_size);
            if last_reported != Some(new_natural) {
                if let Some(viewer) = self.shared_session_viewer_mut() {
                    viewer.last_reported_natural_size = Some(new_natural);
                }
                ctx.emit(Event::ReportViewerTerminalSize {
                    window_size: SessionSharingWindowSize {
                        num_rows: new_natural.0,
                        num_cols: new_natural.1,
                    },
                });
            }
        } else if let Some(viewer) = self.shared_session_viewer_mut() {
            viewer.last_reported_natural_size = None;
        }
    }

    /// This handler is called after *every* terminal view layout with the
    /// size of the entire terminal (block_list + input OR alt-grid OR shared session viewer loading) as its
    /// argument.
    pub(super) fn after_terminal_view_layout(
        &mut self,
        size: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        let size_update = SizeUpdateBuilder::after_layout(*self.size_info, size).build(self, ctx);
        self.resize_internal(size_update, ctx);

        // Update the height of the "gap" - the space we would need to clear
        // in the terminal to accommodate a clear or ctrl-L.
        let mut model = self.model.lock();
        let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
        let gap_height_in_lines = match (
            model.is_alt_screen_active(),
            input_mode,
            model.block_list().active_gap(),
        ) {
            (false, InputMode::Waterfall, Some(_)) => {
                let last_frame_input_height = ctx
                    .element_position_by_id(self.input.as_ref(ctx).save_position_id())
                    .map_or(Pixels::zero(), |r| r.height().into_pixels());

                // If there is already a gap in waterfall mode, we can't use the block list element's size
                // to figure out the next gap and so we have to do some math to figure out
                // the space available for the clear.
                self.size_info.pane_height_px() - last_frame_input_height
            }
            (_, _, _) => {
                // If there is no gap, then the height of the next gap is just the
                // entire height of the block list or alt grid.
                Pixels::new(self.content_element_height_px(ctx))
            }
        }
        .to_lines(self.size_info.cell_height_px);
        model
            .block_list_mut()
            .set_next_gap_height_in_lines(gap_height_in_lines);
    }

    pub(super) fn is_block_visible_locking(
        &self,
        block_index: BlockIndex,
        block_visibility: BlockVisibilityMode,
        input_mode: InputMode,
        app: &AppContext,
    ) -> bool {
        let model = self.model.lock();
        self.is_block_visible(
            block_index,
            model.block_list(),
            block_visibility,
            input_mode,
            app,
        )
    }

    // Whether a block is visible. We define a block to be visible if its command is in the
    // viewport.
    fn is_block_visible(
        &self,
        block_index: BlockIndex,
        block_list: &BlockList,
        block_visibility: BlockVisibilityMode,
        input_mode: InputMode,
        app: &AppContext,
    ) -> bool {
        self.viewport_state(block_list, input_mode, app)
            .is_block_in_view(block_index, block_visibility)
    }

    pub fn mark_as_visible(&mut self) {
        self.was_ever_visible = true;
    }

    pub fn set_active_session_state(
        &mut self,
        _state: ActiveSessionState,
        ctx: &mut ViewContext<Self>,
    ) {
        self.on_pane_state_change(ctx);
    }

    pub(super) fn toggle_left_panel_file_tree(
        &self,
        force_open: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(Event::ToggleLeftPanel {
            target_view: LeftPanelTargetView::FileTree,
            force_open,
        });
    }

    /// Adds persistent toast to toast stack.
    pub fn show_persistent_toast(
        &mut self,
        text: String,
        flavor: ToastFlavor,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::new(text, flavor);
            toast_stack.add_persistent_toast(toast, window_id, ctx);
        });
    }

    /// Adds ephemeral error toast to toast stack.
    pub(super) fn show_error_toast(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::error(text);
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    /// Currently, we show the notification error in the form of a banner,
    /// similar to how we help the user discover notifications via a banner.
    pub fn show_notification_error(
        &mut self,
        error: NotificationSendError,
        ctx: &mut ViewContext<Self>,
    ) {
        // If notifications are not enabled on this platform, we don't want to
        // show the notification error banner.
        if !SessionSettings::as_ref(ctx)
            .notifications
            .is_supported_on_current_platform()
        {
            return;
        }

        self.inline_banners_state.notifications_error_banner.error = Some(error);

        // Only show a banner if it is currently closed
        if matches!(
            self.inline_banners_state
                .notifications_error_banner
                .banner_type,
            NotificationsErrorBannerType::Closed
        ) {
            if self
                .model
                .lock()
                .block_list()
                .active_block()
                .is_active_and_long_running()
            {
                // If the current block is still running, mark the banner as
                // triggered so we can surface it once this block completes
                self.inline_banners_state
                    .notifications_error_banner
                    .banner_type = NotificationsErrorBannerType::Triggered;
            } else {
                // If the current block is not running, open the banner up right away
                self.insert_notifications_error_banner(ctx);
            }
        }
    }

    pub fn scroll_position(&self) -> ScrollPosition {
        self.scroll_position.position()
    }

    pub fn shell_family(&self, ctx: &mut ViewContext<Self>) -> ShellFamily {
        self.active_block_session_id()
            .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id))
            .map(|session| session.shell().shell_type().into())
            .unwrap_or_else(|| {
                SessionSettings::handle(ctx).read(ctx, |settings, _| {
                    settings
                        .new_session_shell_override
                        .value()
                        .clone()
                        .unwrap_or_default()
                        .shell_family()
                })
            })
    }

    pub(super) fn paste(&mut self, middle_click: bool, ctx: &mut ViewContext<Self>) {
        let (should_paste_in_input, needs_bracketed_paste) = {
            let mut model = self.model.lock();
            (
                // If the block list isn't bootstrapped yet, there could be something in the .rc file waiting for input,
                // and we want the paste to go there if the editor isn't focused.
                self.is_input_box_visible(&model, ctx)
                    && (self.input.as_ref(ctx).editor().is_focused(ctx)
                        || model.block_list().is_bootstrapped()),
                model.needs_bracketed_paste(),
            )
        };

        let is_cli_agent_paste =
            !should_paste_in_input && !middle_click && self.has_active_cli_agent_session(ctx);

        // If we're pasting into a CLI coding agent (e.g. Claude Code) that has its own native
        // handling for pasted file paths and images, skip shell-escaping and let the agent
        // see https://github.com/anthropics/claude-code/issues/18590.
        let shell_family = if is_cli_agent_paste {
            None
        } else {
            Some(self.shell_family(ctx))
        };
        let mut copied = if middle_click {
            TerminalView::middle_click_paste_content(shell_family, ctx)
        } else {
            let clipboard_content = ctx.clipboard().read();

            if is_cli_agent_paste && clipboard_content.has_image_data() {
                if !cfg!(windows) {
                    self.write_user_bytes_to_pty(vec![escape_sequences::C0::SYN], ctx);
                    return;
                }

                // On Windows, Claude Code uses Alt+V for native image paste.
                let is_claude = CLIAgentSessionsModel::as_ref(ctx)
                    .session(self.view_id)
                    .is_some_and(|s| s.agent == CLIAgent::Claude);
                if is_claude {
                    self.write_user_bytes_to_pty(vec![escape_sequences::C0::ESC, b'v'], ctx);
                    return;
                }

                // For all other agents on Windows, fall through to the normal paste path. When
                // bracketed paste is enabled (true for TUI-based CLI agents), the empty-text paste
                // sends \x1b[200~\x1b[201~ to the PTY. The agent interprets this as a "paste
                // happened" signal and reads the Windows clipboard directly for image data.
            }

            clipboard_content_with_escaped_paths(clipboard_content, shell_family, false)
        };

        if should_paste_in_input {
            // We put everything from the clipboard into the input box, even
            // if it includes non-printable characters.
            self.input.update(ctx, |input, ctx| {
                input.system_insert(&copied, ctx);
            });
        } else {
            // We need to replace newlines (either \n or \r\n) with \r, as
            // otherwise programs that don't support bracketed paste might
            // misinterpret newlines as a ^J sequence.
            // See: https://github.com/vercel/hyper/issues/1448#issuecomment-367890105
            copied = LINEFEED_REGEX
                .replace_all(copied.as_str(), "\r")
                .to_string();

            if needs_bracketed_paste {
                // If bracketed paste is enabled in the current grid, then we should surround any
                // paste operation with `\x1b[200~` and `\x1b[201~` so that the application knows
                // the text came from paste, rather than from direct user input.
                // See https://cirw.in/blog/bracketed-paste for more info on bracketed paste
                copied = format!("{BRACKETED_PASTE_PREFIX}{copied}{BRACKETED_PASTE_SUFFIX}");
            }
            self.write_user_bytes_to_pty(copied.into_bytes(), ctx);
        }
    }

    pub(super) fn has_active_cli_agent_session(&self, ctx: &AppContext) -> bool {
        CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .is_some()
    }

    pub(super) fn is_inverted_blocklist(&self, ctx: &ViewContext<Self>) -> bool {
        let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
        input_mode.is_inverted_blocklist()
    }

    pub(super) fn copy(&mut self, ctx: &mut ViewContext<Self>) {
        // First check if there's selected text in the CLI subagent views
        for subagent_view in self.cli_subagent_views.values() {
            if let Some(selected_text) = subagent_view.as_ref(ctx).selected_text(ctx) {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(selected_text));
                return;
            }
        }

        // Then check if there's selected text in the cloud mode error screen
        let error_selected_text = self
            .ambient_agent_view_model
            .as_ref()
            .map(|model| model.as_ref(ctx).ui_state.error_selected_text.clone());
        if let Some(error_selected_text) = error_selected_text {
            if let Some(text) = error_selected_text.read().clone().filter(|t| !t.is_empty()) {
                ctx.clipboard().write(ClipboardContent::plain_text(text));
                return;
            }
        }

        let semantic_selection = SemanticSelection::as_ref(ctx);
        if let Some(selected) = self.model.lock().selection_to_string(
            semantic_selection,
            self.is_inverted_blocklist(ctx),
            ctx,
        ) {
            if !selected.is_empty() {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(selected));
            }
            return;
        }

        // Prioritize selected text in the input over selected blocks (APP-4330):
        // it's possible to have both a block and input text selected at the same
        // time, and in that case the user almost always means to copy the input.
        let selected_input_text = self.input.read(ctx, |input, ctx| {
            input
                .editor()
                .read(ctx, |editor, ctx| editor.selected_text(ctx))
        });
        if !selected_input_text.is_empty() {
            ctx.clipboard()
                .write(ClipboardContent::plain_text(selected_input_text));
            return;
        }

        if !self.selected_blocks.is_empty() {
            self.copy_blocks(BlockEntity::CommandAndOutput, ctx);
        }
    }

    pub(super) fn copy_commands(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.selected_blocks.is_empty() {
            self.copy_blocks(BlockEntity::Command, ctx);
        }
    }

    pub(super) fn copy_outputs(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.selected_blocks.is_empty() {
            self.copy_blocks(BlockEntity::FilteredOutput, ctx);
        }
    }

    /// Returns the rich-content link currently hovered inside the AI block view whose view id is
    /// `rich_content_view_id`, if any. Used to surface a link-specific right-click context menu.
    pub(super) fn hovered_rich_content_link_for_view(
        &self,
        rich_content_view_id: EntityId,
        ctx: &AppContext,
    ) -> Option<RichContentLink> {
        self.ai_block_handle_by_view_id(rich_content_view_id)?
            .as_ref(ctx)
            .hovered_rich_content_link()
    }
}
