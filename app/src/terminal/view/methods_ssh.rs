use super::*;

impl TerminalView {
    pub(super) fn on_ssh_warpification_key_event(
        &mut self,
        key_event: Option<SshKeyEvent>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.warpify_state.ssh_block_state().is_some() {
            if key_event.is_some_and(|key| key.is_ctrl_c()) {
                send_telemetry_from_ctx!(TelemetryEvent::SshTmuxWarpifyBlockDismissed, ctx);
                self.cancel_bootstrap_workflow(ctx);
            } else if self.warpify_state.should_prevent_input() {
                self.warpify_state.focus(ctx);
                self.warpify_state.collapse_ssh_block(ctx);
                self.update_scroll_position_locking(
                    ScrollPositionUpdate::AfterRichBlockUpdated,
                    ctx,
                );
                ctx.notify();
            }
        }
    }

    pub(super) fn handle_remote_warpification_is_unavailable(
        &mut self,
        reason: WarpificationUnavailableReason,
        ctx: &mut ViewContext<Self>,
    ) {
        // Stop the pending timeout on warpification.
        self.warpify_state.abort_ssh_warpify_timeout();
        match &reason {
            WarpificationUnavailableReason::TmuxNotInstalled {
                system_details,
                root_access,
            } => {
                if system_details.writable_home != Some(true) {
                    if let Some(shell_type) = ShellType::from_name(&system_details.shell) {
                        self.trigger_subshell_bootstrap(Some(shell_type), false, ctx);
                        return;
                    }
                }

                if let Some(tmux_install_script) = install_tmux_script(system_details, ctx) {
                    let root_access = RootAccess::from_str(root_access).unwrap_or_default();
                    let tmux_root_install_script = if root_access == RootAccess::NoRootAccess {
                        None
                    } else {
                        install_root_tmux_script(
                            system_details,
                            ctx,
                            root_access == RootAccess::CanRunSudo,
                        )
                    };
                    self.add_ssh_install_tmux_block(
                        system_details,
                        tmux_install_script,
                        tmux_root_install_script,
                        false,
                        ctx,
                    );
                    return;
                }
            }
            WarpificationUnavailableReason::UnsupportedTmuxVersion { system_details } => {
                if system_details.writable_home != Some(true) {
                    if let Some(shell_type) = ShellType::from_name(&system_details.shell) {
                        self.trigger_subshell_bootstrap(Some(shell_type), false, ctx);
                        return;
                    }
                }

                if let Some(tmux_install_script) = install_tmux_script(system_details, ctx) {
                    self.add_ssh_install_tmux_block(
                        system_details,
                        tmux_install_script,
                        None,
                        true,
                        ctx,
                    );
                    return;
                }
            }
            _ => {}
        }
        self.add_ssh_error_block(reason, ctx);
    }

    pub(super) fn add_ssh_warpify_prompt(
        &mut self,
        command: &str,
        ssh_host: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.clear_ssh_blocks(ctx);
        self.handle_action(
            &TerminalAction::ShowWarpifySshBanner(command.to_owned(), ssh_host),
            ctx,
        );
    }

    /// This method assumes the active block in the blocklist is a long-running SSH command.
    pub(super) fn add_ssh_warpifying_block(&mut self, ctx: &mut ViewContext<Self>) {
        // Shared session viewers can't initiate warpification currently.
        if self.model.lock().shared_session_status().is_viewer() {
            return;
        }

        self.clear_ssh_blocks(ctx);

        let show_ssh_block_debug = BlockVisibilitySettings::as_ref(ctx)
            .should_show_ssh_block
            .value();
        let (full_ssh_command, hidden_ssh_block_id) = {
            let mut model = self.model.lock();
            if !show_ssh_block_debug {
                model.block_list_mut().active_block_mut().hide();
            }

            (
                model.block_list().active_block().command_to_string(),
                model.block_list().active_block_id().clone(),
            )
        };

        let ssh_warpify_block_handle =
            ctx.add_typed_action_view(|_| SshWarpifyBlock::new(full_ssh_command));
        ctx.subscribe_to_view(&ssh_warpify_block_handle, move |me, _, event, ctx| {
            me.handle_ssh_warpify_block_event(event, ctx);
        });

        self.insert_rich_content(
            None,
            ssh_warpify_block_handle.clone(),
            Some(RichContentMetadata::SshWarpifyBlock {
                ssh_warpify_block_handle: ssh_warpify_block_handle.clone(),
            }),
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: true,
            },
            ctx,
        );

        ctx.focus(&ssh_warpify_block_handle);

        self.warpify_state.set_block_id(hidden_ssh_block_id);
        self.warpify_state
            .set_ssh_block_state(SshBlockState::Warpifying {
                handle: ssh_warpify_block_handle,
            });

        self.warpify_ssh_session(ctx);
    }

    /// This method assumes the active block in the blocklist is a long-running SSH command.
    pub(super) fn add_ssh_install_tmux_block(
        &mut self,
        system_details: &SystemDetails,
        tmux_install_script: String,
        tmux_root_install_script: Option<String>,
        outdated_version: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.clear_ssh_blocks(ctx);

        let show_ssh_block_debug = BlockVisibilitySettings::as_ref(ctx)
            .should_show_ssh_block
            .value();
        let (full_ssh_command, hidden_ssh_block_id) = {
            let mut model = self.model.lock();
            if !show_ssh_block_debug {
                model.block_list_mut().active_block_mut().hide();
            }
            (
                model.block_list().active_block().command_to_string(),
                model.block_list().active_block_id().clone(),
            )
        };

        let ssh_host = self.warpify_state.get_pending_ssh_host();

        let ssh_install_tmux_block_handle = ctx.add_typed_action_view(|_| {
            SshInstallTmuxBlock::new(
                system_details.clone(),
                tmux_install_script,
                tmux_root_install_script,
                full_ssh_command,
                ssh_host,
                outdated_version,
            )
        });
        ctx.subscribe_to_view(&ssh_install_tmux_block_handle, move |me, _, event, ctx| {
            me.handle_ssh_install_tmux_block_event(event, ctx);
        });

        self.insert_rich_content(
            None,
            ssh_install_tmux_block_handle.clone(),
            Some(RichContentMetadata::SshInstallTmuxBlock {
                ssh_install_tmux_block_handle: ssh_install_tmux_block_handle.clone(),
            }),
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: true,
            },
            ctx,
        );

        ctx.focus(&ssh_install_tmux_block_handle);

        send_telemetry_from_ctx!(TelemetryEvent::SshInstallTmuxBlockDisplayed, ctx);

        self.warpify_state.set_block_id(hidden_ssh_block_id);
        self.warpify_state
            .set_ssh_block_state(SshBlockState::InstallTmux {
                handle: ssh_install_tmux_block_handle,
            });
    }

    pub(super) fn add_ssh_error_block(
        &mut self,
        error_reason: WarpificationUnavailableReason,
        ctx: &mut ViewContext<Self>,
    ) {
        // If there's already an error block showing, don't overwrite the existing one.
        if matches!(
            self.warpify_state.ssh_block_state(),
            Some(SshBlockState::Error { .. })
        ) {
            return;
        }

        self.clear_ssh_blocks(ctx);
        self.update_long_running_ssh_block_with_lock(|block| {
            block.unhide();
        });

        let ssh_host = self.warpify_state.take_pending_ssh_host();

        let ssh_error_block_handle =
            ctx.add_typed_action_view(|_| SshErrorBlock::new(error_reason.clone(), ssh_host));
        ctx.subscribe_to_view(&ssh_error_block_handle, move |me, _, event, ctx| {
            me.handle_ssh_error_block_events(event, ctx);
        });

        self.insert_rich_content(
            None,
            ssh_error_block_handle.clone(),
            Some(RichContentMetadata::SshErrorBlock {
                ssh_error_block_handle: ssh_error_block_handle.clone(),
            }),
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: true,
            },
            ctx,
        );

        send_telemetry_from_ctx!(
            TelemetryEvent::SshTmuxWarpificationErrorBlock {
                error: error_reason,
                tmux_installation: self.warpify_state.tmux_installation(),
            },
            ctx
        );

        self.warpify_state
            .set_ssh_block_state(SshBlockState::Error {
                handle: ssh_error_block_handle,
            });
        self.warpify_state.focus(ctx);
    }

    pub(super) fn add_bootstrap_success_block(
        &mut self,
        SessionBootstrappedEvent {
            spawning_command,
            subshell_info,
            shell,
            session_type,
            ..
        }: SessionBootstrappedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let show_ssh_block_debug = BlockVisibilitySettings::as_ref(ctx)
            .should_show_ssh_block
            .value();
        if !show_ssh_block_debug {
            self.update_long_running_ssh_block_with_lock(|block| {
                block.hide();
            });
        }

        let warpification_source = match session_type {
            BootstrapSessionType::WarpifiedRemote => WarpificationSource::Ssh,
            BootstrapSessionType::Local => WarpificationSource::Subshell,
        };
        let disable_tmux = FeatureFlag::SSHTmuxWrapper.is_enabled()
            && matches!(warpification_source, WarpificationSource::Ssh)
            && { !self.model.lock().tmux_control_mode_active() };
        let ssh_success_block_handle = ctx.add_typed_action_view(|ctx| {
            WarpifySuccessBlock::new(
                warpification_source,
                spawning_command,
                subshell_info,
                shell,
                disable_tmux,
                ctx,
            )
        });
        ctx.subscribe_to_view(&ssh_success_block_handle, move |me, _, event, ctx| {
            me.handle_ssh_success_block_events(event, ctx);
        });

        self.clear_ssh_blocks(ctx);
        self.insert_rich_content(
            Some(RichContentType::WarpifySuccessBlock),
            ssh_success_block_handle.clone(),
            Some(RichContentMetadata::WarpifySuccessBlock {
                bootstrap_success_block_handle: ssh_success_block_handle.clone(),
            }),
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: false,
            },
            ctx,
        );
        self.warpify_state
            .set_ssh_block_state(SshBlockState::WarpifySuccess {
                handle: ssh_success_block_handle,
            });
        let active_session_id = self.active_block_session_id();
        self.warpify_state.on_warpify_start(active_session_id);
        self.refresh_warp_prompt(ctx);
    }

    pub(super) fn handle_ssh_warpify_block_event(
        &mut self,
        event: &SshWarpifyBlockEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        fn dismiss_ssh_warpify_block(me: &mut TerminalView, ctx: &mut ViewContext<TerminalView>) {
            send_telemetry_from_ctx!(TelemetryEvent::SshTmuxWarpifyBlockDismissed, ctx);
            me.cancel_bootstrap_workflow(ctx);
        }

        match event {
            SshWarpifyBlockEvent::Cancel => {
                self.warpify_state.replace_timeout_id();
                dismiss_ssh_warpify_block(self, ctx);
            }
            SshWarpifyBlockEvent::Interrupt => {
                dismiss_ssh_warpify_block(self, ctx);
                self.warpify_state.abort_ssh_warpify_timeout();
                self.user_write_ctrl_c_to_pty(ctx);
            }
            SshWarpifyBlockEvent::WarpifySession => {
                send_telemetry_from_ctx!(TelemetryEvent::SshTmuxWarpifyBlockAccepted, ctx);
                self.add_ssh_warpifying_block(ctx);
                self.update_scroll_position_locking(
                    ScrollPositionUpdate::AfterRichBlockUpdated,
                    ctx,
                );
                ctx.notify();
            }
        }
    }

    pub(super) fn handle_ssh_install_tmux_block_event(
        &mut self,
        event: &SshInstallTmuxBlockEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        fn cancel_tmux_install(me: &mut TerminalView, ctx: &mut ViewContext<TerminalView>) {
            send_telemetry_from_ctx!(TelemetryEvent::SshInstallTmuxBlockDismissed, ctx);
            me.cancel_bootstrap_workflow(ctx);
        }

        match event {
            SshInstallTmuxBlockEvent::Cancel => {
                cancel_tmux_install(self, ctx);
            }
            SshInstallTmuxBlockEvent::Interrupt => {
                cancel_tmux_install(self, ctx);
                self.warpify_state.abort_ssh_warpify_timeout();
                self.user_write_ctrl_c_to_pty(ctx);
            }
            SshInstallTmuxBlockEvent::InstallTmuxAndWarpify(install_source) => {
                send_telemetry_from_ctx!(TelemetryEvent::SshInstallTmuxBlockAccepted, ctx);
                self.clear_ssh_blocks(ctx);
                self.install_tmux_and_warpify(ctx, install_source);
                self.update_scroll_position_locking(
                    ScrollPositionUpdate::AfterRichBlockUpdated,
                    ctx,
                );
                ctx.notify();
            }
            SshInstallTmuxBlockEvent::ToggleScriptVisibility => {
                self.update_scroll_position_locking(
                    ScrollPositionUpdate::AfterRichBlockUpdated,
                    ctx,
                );
                ctx.notify();
            }
            SshInstallTmuxBlockEvent::ToggleTmuxInstallVisibility => {
                if let Some(ssh_block_id) = self.warpify_state.block_id() {
                    if let Some(is_visible) = self
                        .model
                        .lock()
                        .block_list_mut()
                        .toggle_visibility_of_block(&ssh_block_id)
                    {
                        if is_visible {
                            ctx.focus_self();
                        }
                    }
                    ctx.notify();
                }
            }
            SshInstallTmuxBlockEvent::UnhideTmuxInstall => {
                if let Some(ssh_block_id) = self.warpify_state.block_id() {
                    self.model
                        .lock()
                        .block_list_mut()
                        .unhide_block(&ssh_block_id);
                    ctx.focus_self();
                    ctx.notify();
                }
            }
        }
    }

    pub(super) fn handle_ssh_error_block_events(
        &mut self,
        event: &SshErrorBlockEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SshErrorBlockEvent::WarpifyWithoutTmux => {
                let shell_type = self.warpify_state.get_shell_type();
                self.clear_ssh_blocks(ctx);
                self.trigger_subshell_bootstrap(shell_type, false, ctx);
            }
            SshErrorBlockEvent::ContinueWithoutWarpification => {
                self.cancel_bootstrap_workflow(ctx);
            }
        }
    }

    pub(super) fn handle_ssh_success_block_events(
        &mut self,
        event: &WarpifySuccessBlockEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            WarpifySuccessBlockEvent::OpenWarpifySettings => {
                ctx.emit(Event::OpenSettings(SettingsSection::Warpify));
            }
        }
    }

    pub(super) fn dismiss_warpify_banner(
        &mut self,
        remember_command: &RememberForWarpification,
        ctx: &mut ViewContext<Self>,
    ) {
        {
            let mut model = self.model.lock();
            model.block_list_mut().set_active_block_banner(None);
        }

        // Also clear the warpify footer so it doesn't linger after warpification
        // starts, fails, or is cancelled.
        if FeatureFlag::WarpifyFooter.is_enabled() {
            self.use_agent_footer.update(ctx, |footer, ctx| {
                footer.clear_warpify_mode(ctx);
            });
        }

        match remember_command {
            RememberForWarpification::RememberSubshellCommand(command) => {
                WarpifySettings::handle(ctx).update(ctx, |warpify, ctx| {
                    warpify.denylist_subshell_command(command, ctx);
                });
            }
            RememberForWarpification::RememberSSHHost(host) => {
                WarpifySettings::handle(ctx).update(ctx, |warpify, ctx| {
                    warpify.denylist_ssh_host(host, ctx);
                });
            }
            RememberForWarpification::DoNotRememberSubshellCommand
            | RememberForWarpification::DoNotRememberSSHHost => {}
        }
    }

    pub(super) fn show_warpify_banner(
        &mut self,
        input: WarpificationMode,
        title: &str,
        lowercase_title: &str,
        warpify_keybinding: Option<Keystroke>,
        telemetry_event: TelemetryEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if FeatureFlag::WarpifyFooter.is_enabled() {
            return;
        }

        let mut model = self.model.lock();

        // Shared session viewers can't initiate warpification currently.
        // Don't show the warpify banner when an agent is monitoring the command either.
        if model.shared_session_status().is_viewer()
            || model.block_list().active_block().is_agent_monitoring()
        {
            return;
        }

        let a11y_message = match &warpify_keybinding {
            Some(keystroke) => format!(
                "You can press {} to Warpify this {} for more Warp features.",
                keystroke.displayed(),
                lowercase_title
            ),
            None => format!("You can Warpify this {lowercase_title} for more Warp features."),
        };

        model
            .block_list_mut()
            .set_active_block_banner(Some(WithinBlockBanner::WarpifyBanner(
                WarpifyBannerState::new(input, warpify_keybinding),
            )));

        let a11y_content = AccessibilityContent::new(
            format!("{title} recognized."),
            a11y_message,
            WarpA11yRole::TextRole,
        );
        ctx.emit_a11y_content(a11y_content);

        send_telemetry_from_ctx!(telemetry_event, ctx);

        ctx.notify();
    }

    pub(super) fn insert_most_recent_command_correction(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(most_recent_command_correction) = self.most_recent_command_correction.as_ref() {
            self.input.update(ctx, |input, ctx| {
                input.replace_buffer_content(most_recent_command_correction.command.as_str(), ctx);
                ctx.notify()
            });

            send_telemetry_from_ctx!(
                TelemetryEvent::CommandCorrection {
                    event: CommandCorrectionEvent::Accepted {
                        via: CommandCorrectionAcceptedType::Keybinding,
                        rule: most_recent_command_correction.rule_applied.to_str(),
                    }
                },
                ctx
            );
        }
    }

    pub(super) fn alias_expansion_banner_action(
        &mut self,
        action: AliasExpansionBannerAction,
        ctx: &mut ViewContext<Self>,
    ) {
        use AliasExpansionBannerAction::*;

        match action {
            Enable => {
                let mut should_dismiss_banner = true;
                AliasExpansionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    if let Err(e) = settings.alias_expansion_enabled.set_value(true, ctx) {
                        should_dismiss_banner = false;
                        log::error!("Failed to enable alias expansion setting from banner: {e}");
                    }
                });
                if should_dismiss_banner {
                    self.dismiss_alias_expansion_banner(ctx);
                    send_telemetry_from_ctx!(TelemetryEvent::EnableAliasExpansionFromBanner, ctx);
                }
            }
            Dismiss => {
                self.dismiss_alias_expansion_banner(ctx);
                send_telemetry_from_ctx!(TelemetryEvent::DismissAliasExpansionBanner, ctx);
            }
        };
    }

    pub(super) fn dismiss_alias_expansion_banner(&mut self, ctx: &mut ViewContext<Self>) {
        if let AliasExpansionBanner::Open { state } =
            &self.inline_banners_state.alias_expansion_banner
        {
            self.model
                .lock()
                .block_list_mut()
                .remove_inline_banner(state.id);
            self.inline_banners_state.alias_expansion_banner = AliasExpansionBanner::Closed;
        }
        ctx.notify();
    }
}
