use super::*;

impl TerminalView {
    pub(super) fn on_user_block_completed(
        &mut self,
        block_id: &BlockId,
        ctx: &mut ViewContext<Self>,
    ) {
        {
            self.model
                .lock()
                .clear_pending_warp_initiated_control_mode();
        }
        self.model.lock().end_notify_on_ssh_login_complete();

        // If the block that just ended was an agent-requested long running command for which the user took over control,
        // and the user exited the command, we should resume the conversation.
        let conversation_id_to_resume = {
            let model = self.model.lock();
            let ai_metadata = model
                .block_list()
                .block_with_id(block_id)
                .and_then(|block| block.agent_interaction_metadata());

            match ai_metadata {
                Some(ai_metadata)
                    if ai_metadata.requested_command_action_id().is_some()
                        && ai_metadata
                            .long_running_control_state()
                            .is_some_and(|state| {
                                state
                                    .user_take_over_reason()
                                    .is_some_and(|reason| !reason.is_stop())
                            }) =>
                {
                    Some(*ai_metadata.conversation_id())
                }
                _ => None,
            }
        };

        if let Some(conversation_id) = conversation_id_to_resume {
            // Include the context of the block that just completed in the resume context.
            // This is so that we correctly exit from LRC subagents attached to completed commands.
            let resume_context = {
                let terminal_model = self.model.lock();
                block_context_from_terminal_model(&terminal_model, block_id, false)
                    .map(Box::new)
                    .map(AIAgentContext::Block)
                    .into_iter()
                    .collect()
            };

            self.ai_controller.update(ctx, |controller, ctx| {
                controller.resume_conversation(
                    conversation_id,
                    /*can_attempt_resume_on_error*/ true,
                    /*is_auto_resume_after_error*/ false,
                    resume_context,
                    ctx,
                );
            });
        }

        // Hide telemetry banner forever after first block user executes.
        if FeatureFlag::GlobalAIAnalyticsBanner.is_enabled()
            && !GeneralSettings::as_ref(ctx)
                .telemetry_banner_dismissed
                .value()
        {
            self.hide_telemetry_banner_permanently(ctx);
        }
    }

    pub(super) fn active_block_is_considered_remote(&self, app: &AppContext) -> bool {
        let model = self.model.lock();
        let active_block = model.block_list().active_block();
        self.is_block_considered_remote(
            active_block.session_id(),
            Some(&active_block.command_to_string()),
            app,
        )
    }

    /// Returns true if the block is considered remote.
    ///
    /// Note that we don't know for sure if a block is remote, because we can only detect
    /// warpified remote blocks.
    ///
    /// For some organizations, we accept a regex list that we run against commands to
    /// further make the determination.
    pub(super) fn is_block_considered_remote(
        &self,
        session_id: Option<SessionId>,
        command: Option<&str>,
        app: &AppContext,
    ) -> bool {
        let is_warpified_remote = session_id
            .map(|id| {
                self.sessions
                    .as_ref(app)
                    .get(id)
                    .map(|session| !session.is_local())
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        if is_warpified_remote {
            return true;
        }

        // If there's a command present and this user is subject to the regex list policy from their
        // organization, check the command against the regex list.

        let Some(command) = command else {
            return false;
        };

        if UserWorkspaces::as_ref(app).is_ai_allowed_in_remote_sessions() {
            // We don't check any regexes if the user is allowed to run AI in remote sessions.
            return false;
        }

        let remote_session_regex_list = UserWorkspaces::as_ref(app).get_remote_session_regex_list();

        // First check if the command matches any of the regexes in the list.
        if remote_session_regex_list
            .iter()
            .any(|regex| regex.is_match(command))
        {
            return true;
        }

        // Then check if there's an alias for the top level command that matches the regex.
        let Some(session_id) = session_id else {
            return false;
        };
        let Some(session) = self.sessions.as_ref(app).get(session_id) else {
            return false;
        };
        let escape_char = session.shell_family().escape_char();
        let Some(top_level_command) =
            warp_completer::parsers::simple::top_level_command(command, escape_char)
        else {
            return false;
        };
        let Some(alias) = session.alias_value(top_level_command.as_str()) else {
            return false;
        };

        if remote_session_regex_list
            .iter()
            .any(|regex| regex.is_match(alias))
        {
            return true;
        }

        false
    }

    // Abort any pending prompt or code suggestions, which may now be irrelevant.
    pub(super) fn abort_prompt_and_code_suggestions(&mut self, ctx: &mut ViewContext<Self>) {
        // Abort both models to handle any in-flight requests from before a
        // feature flag change.
        self.passive_suggestions_models
            .maa
            .update(ctx, |model, ctx| model.abort_pending_requests(ctx));
        let pending_stream_ids = self
            .passive_suggestions_models
            .legacy
            .update(ctx, |model, ctx| model.abort_pending_requests(ctx));
        for stream_id in pending_stream_ids {
            if let Some(passive_block) =
                self.rich_content_views
                    .iter()
                    .rev()
                    .find_map(|rich_content| {
                        let ai_metadata = rich_content.ai_block_metadata()?;
                        if ai_metadata
                            .ai_block_handle
                            .as_ref(ctx)
                            .response_stream_id()
                            .is_some_and(|id| id == &stream_id)
                        {
                            return Some(ai_metadata.ai_block_handle.clone());
                        }
                        None
                    })
            {
                self.cleanup_and_remove_conversation_for_ai_block(&passive_block, ctx);
            }
        }
    }

    /// Cleans up and removes the conversation associated with the given AI block.
    ///
    /// This removes the AI block from the blocklist (and cached `rich_content_views` list) and
    /// deletes its associated conversation.
    ///
    /// This assumes that the deleted conversation only contains a single block -- should there
    /// be other blocks besides `passive_block` in the same conversation, we're left in invalid
    /// state (AI blocks rely on conversation state in the history model to render). If there is
    /// more than one AI block corresponding to the same conversation as `passive_block`, does
    /// nothing.
    pub(super) fn cleanup_and_remove_conversation_for_ai_block(
        &mut self,
        passive_block: &ViewHandle<AIBlock>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some((rich_content_idx, _)) =
            self.rich_content_views
                .iter()
                .enumerate()
                .find_map(|(idx, rich_content)| {
                    let ai_metadata = rich_content.ai_block_metadata()?;
                    (ai_metadata.ai_block_handle.id() == passive_block.id())
                        .then(|| (idx, ai_metadata.ai_block_handle.clone()))
                })
        else {
            return;
        };

        let has_other_blocks_in_same_conversation =
            self.rich_content_views.iter().any(|rich_content| {
                if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                    ai_metadata.ai_block_handle.id() != passive_block.id()
                        && ai_metadata.ai_block_handle.as_ref(ctx).conversation_id()
                            == passive_block.as_ref(ctx).conversation_id()
                } else {
                    false
                }
            });
        if has_other_blocks_in_same_conversation {
            log::error!(
                "Attempted to clean up and delete conversation for block with other blocks in the same conversation"
            );
            return;
        }

        passive_block.update(ctx, |ai_block, ctx| {
            ai_block.cleanup_block(ctx);
        });
        let conversation_id = passive_block.as_ref(ctx).conversation_id();
        conversation_utils::remove_conversation(conversation_id, self.view_id, true, ctx);
        self.rich_content_views.remove(rich_content_idx);
        self.model
            .lock()
            .block_list_mut()
            .remove_rich_content(passive_block.id());
        ctx.notify();
    }

    /// Sends telemetry if an AI-requested command caused the shell to exit.
    pub(super) fn maybe_send_agent_exited_shell_telemetry(&self, ctx: &mut ViewContext<Self>) {
        let model = self.model.lock();
        let block_list = model.block_list();
        let blocks = block_list.blocks();
        let active_block_index = block_list.active_block_index().0;

        // There are two cases we need to handle:
        // 1. The agent ran a shell command that directly exits the shell process
        //    (e.g. `exit 1`). The requested command will be the active block.
        // 2. The shell is in a state where it can choose to exit itself after a command
        //    finishes as part of its command execution loop (e.g. earlier it ran
        //    `set -euo pipefail`). In this case the requested command will be in the
        //    block preceding the active block.
        let agent_block = blocks
            .get(active_block_index)
            .filter(|b| b.requested_command_action_id().is_some())
            .or_else(|| {
                active_block_index.checked_sub(1).and_then(|prev_idx| {
                    blocks
                        .get(prev_idx)
                        .filter(|b| b.requested_command_action_id().is_some())
                })
            });

        if let Some(block) = agent_block {
            let mut command = block.command_to_string();
            redact_secrets(&mut command);

            let server_output_id = block.ai_conversation_id().and_then(|conversation_id| {
                BlocklistAIHistoryModel::as_ref(ctx)
                    .conversation(&conversation_id)
                    .and_then(|conversation| {
                        conversation
                            .latest_exchange()
                            .and_then(|e| e.output_status.server_output_id())
                    })
            });
            send_telemetry_from_ctx!(
                TelemetryEvent::AgentExitedShellProcess {
                    command,
                    server_output_id,
                },
                ctx
            );
        }
    }

    /// Updates the back button's state and label. For child agents the
    /// label becomes "for Orchestrator" since ESC swaps to the parent
    /// instead of exiting in place.
    pub(crate) fn update_agent_view_back_button_state(&mut self, ctx: &mut ViewContext<Self>) {
        let active_conv_id = self
            .agent_view_controller
            .as_ref(ctx)
            .agent_view_state()
            .active_conversation_id();
        let is_child_agent = active_conv_id
            .and_then(|id| BlocklistAIHistoryModel::as_ref(ctx).conversation(&id))
            .and_then(|c| c.parent_conversation_id())
            .is_some();

        // Never disable for child agents: the swap-back path can't be blocked.
        let disabled_reason = if is_child_agent {
            None
        } else {
            self.agent_view_controller
                .as_ref(ctx)
                .can_exit_agent_view()
                .err()
                .map(|e| e.to_string())
        };
        let label = if is_child_agent {
            "for Orchestrator"
        } else {
            "for terminal"
        };

        self.agent_view_back_button.update(ctx, |button, ctx| {
            button.set_label(label, ctx);
            button.set_disabled(disabled_reason.is_some(), ctx);
            button.set_tooltip(disabled_reason, ctx);
        });
    }

    /// Apply a block metadata update from either the precmd hook
    /// ([`Event::BlockMetadataReceived`]) or an OSC 7 sequence emitted
    /// mid-block ([`Event::BlockWorkingDirectoryUpdated`]). The `source`
    /// controls work that's safe to do once per block but wrong to do
    /// repeatedly mid-block — see [`BlockMetadataUpdateSource`].
    pub(super) fn apply_block_metadata_update(
        &mut self,
        block_metadata: &BlockMetadata,
        is_after_in_band_command: bool,
        is_done_bootstrapping: bool,
        source: BlockMetadataUpdateSource,
        ctx: &mut ViewContext<Self>,
    ) {
        // In-band commands don't change the CWD, git state, or session
        // metadata. Skip the expensive processing (git repo detection,
        // directory indexing, re-renders) to avoid an infinite loop where
        // a re-render triggers completions which fire another in-band
        // command. See also the complementary guard in
        // Input::set_active_block_metadata.
        if is_after_in_band_command {
            self.active_block_metadata = Some(block_metadata.clone());
            self.input.update(ctx, |view, ctx| {
                view.set_active_block_metadata(
                    block_metadata.clone(),
                    true, // is_after_in_band_command
                    ctx,
                );
            });
            return;
        }

        if let Some(prev_block_metadata) = self.active_block_metadata.take() {
            // Only send event to save app state when the block is post bootstrap
            // and working directory has changed.
            if prev_block_metadata.current_working_directory()
                != block_metadata.current_working_directory()
                && is_done_bootstrapping
            {
                ctx.emit(Event::AppStateChanged);
            }

            // Update the shell launch data for the active session.
            if prev_block_metadata.session_id() != block_metadata.session_id()
                && is_done_bootstrapping
            {
                let shell_launch_data = self.shell_launch_data_if_local(ctx);
                self.on_active_shell_launch_data_updated(shell_launch_data, ctx);
            }

            // Check if the block is done bootstrapping and the directory is set.
            if let Some(active_directory) = block_metadata.current_working_directory() {
                // See `BlockMetadataUpdateSource` for why OSC 7 needs the
                // CWD-changed gate; precmd keeps its once-per-block semantics.
                let should_run_detection = match source {
                    BlockMetadataUpdateSource::Precmd => true,
                    BlockMetadataUpdateSource::Osc7 => {
                        prev_block_metadata.current_working_directory()
                            != block_metadata.current_working_directory()
                    }
                };
                if is_done_bootstrapping && should_run_detection {
                    // Derive locality directly from the incoming block's
                    // session_id. We cannot use `active_session_is_local(ctx)`
                    // here because `active_block_metadata` was just consumed
                    // via `take()` above, so it would always return `None`
                    // and misclassify every local session as Remote.
                    //
                    // `session_is_local` keeps the shared-session viewer /
                    // conversation-transcript guard intact.
                    let session_id = block_metadata.session_id();
                    let session_type = session_id.map(|sid| {
                        if self.session_is_local(sid, ctx) {
                            RepoDetectionSessionType::Local
                        } else {
                            RepoDetectionSessionType::Remote { session_id: sid }
                        }
                    });
                    if let Some(session_type) = session_type {
                        let is_local = matches!(session_type, RepoDetectionSessionType::Local);

                        // For local sessions, convert the shell-native CWD
                        // (e.g. "/c/Users/..." for Git Bash/MSYS2) to a
                        // Windows-native path before repo detection.
                        let directory_for_detection = if is_local {
                            block_metadata
                                .session_id()
                                .and_then(|sid| self.sessions.as_ref(ctx).get(sid))
                                .and_then(|session| {
                                    session.launch_data().and_then(|data| {
                                        data.maybe_convert_absolute_path(active_directory)
                                    })
                                })
                                .map(|path| path.to_string_lossy().into_owned())
                                .unwrap_or_else(|| active_directory.to_string())
                        } else {
                            active_directory.to_string()
                        };

                        let fut = detect_possible_git_repo(
                            session_type,
                            &directory_for_detection,
                            RepoDetectionSource::TerminalNavigation,
                            ctx,
                        );

                        ctx.spawn(fut, move |me, repo_path_opt, ctx| {
                            let old_repo_path = me.current_repo_path.clone();
                            me.current_repo_path = repo_path_opt.clone();

                            if old_repo_path != me.current_repo_path {
                                ctx.emit(Event::Pane(PaneEvent::RepoChanged));
                            }

                            // `block_completed_callbacks` are scheduled via
                            // `on_next_block_completed` and expect the block
                            // to have finished. OSC 7 fires mid-block, so
                            // draining them here would run callbacks like
                            // `maybe_set_pending_repo_init_path`'s project
                            // init before the actual command (e.g. `git
                            // clone`) finishes.
                            if matches!(source, BlockMetadataUpdateSource::Precmd) {
                                let callbacks =
                                    me.block_completed_callbacks.drain(..).collect_vec();
                                for callback in callbacks {
                                    callback(me, ctx);
                                }
                            }

                            match &repo_path_opt {
                                Some(LocalOrRemotePath::Remote(remote_path)) => {
                                    #[cfg(not(target_family = "wasm"))]
                                    DetectedRepositories::handle(ctx).update(
                                        ctx,
                                        |repos, _| {
                                            repos.register_remote_repo_root(remote_path.clone());
                                        },
                                    );

                                    // Remote sessions can only materialize their working
                                    // directory after repo detection has resolved the host.
                                    // Re-run app-state propagation now that the remote path
                                    // is known so the active session's working directory catches up.
                                    ctx.emit(Event::AppStateChanged);

                                    if FeatureFlag::AIContextMenuEnabled.is_enabled() {
                                        me.input.update(ctx, |input, ctx| {
                                            input
                                                .check_and_update_ai_context_menu_disabled_state(
                                                    ctx,
                                                );
                                        });
                                    }
                                    ctx.emit(Event::Pane(PaneEvent::RemoteRepoNavigated {
                                        remote_path: remote_path.clone(),
                                    }));
                                }
                                Some(LocalOrRemotePath::Local(repo_path)) => {
                                    #[cfg(feature = "local_fs")]
                                    {
                                        let Some(active_directory) =
                                            me.active_session_path_if_local(ctx)
                                        else {
                                            me.clear_git_repo_status(ctx);
                                            return;
                                        };

                                        let Ok(active_directory) =
                                            repo_metadata::CanonicalizedPath::try_from(
                                                active_directory,
                                            )
                                        else {
                                            return;
                                        };

                                        let is_ancestor = active_directory
                                            .as_path_buf()
                                            .ancestors()
                                            .any(|ancestor| ancestor == repo_path.as_path());
                                        if !is_ancestor {
                                            return;
                                        }

                                        PersistedWorkspace::handle(ctx).update(
                                            ctx,
                                            |manager, _| {
                                                manager.navigated_to_path(
                                                    active_directory.as_path_buf(),
                                                );
                                            },
                                        );

                                        if old_repo_path
                                            .as_ref()
                                            .and_then(|p| p.to_local_path())
                                            != Some(repo_path.as_path())
                                        {
                                                me.clear_git_repo_status_subscription(ctx);
                                            me.update_git_status_subscription(ctx);
                                        }

                                        me.input.update(ctx, |input, ctx| {
                                            input.update_repo_path(
                                                Some(repo_path.clone()),
                                                ctx,
                                            );
                                        });

                                        if FeatureFlag::AIContextMenuEnabled.is_enabled() {
                                            me.input.update(ctx, |input, ctx| {
                                                input
                                                    .check_and_update_ai_context_menu_disabled_state(
                                                        ctx,
                                                    );
                                            });
                                        }

                                        me.start_lsp_server_in_active_pwd(ctx);

                                        me.update_repo_banner_state(repo_path.clone(), ctx);
                                    }
                                    #[cfg(not(feature = "local_fs"))]
                                    let _ = repo_path;
                                }
                                None => {
                                    #[cfg(feature = "local_fs")]
                                    me.clear_git_repo_status(ctx);
                                    ctx.notify();
                                }
                            }
                        });
                    }
                }
            }
        }

        self.active_block_metadata = Some(block_metadata.clone());

        if let Some(session) = block_metadata
            .session_id()
            .and_then(|id| self.sessions.as_ref(ctx).get(id))
        {
            let shell_host = ShellHost::from_session(session.as_ref());
            self.model
                .lock()
                .block_list_mut()
                .set_active_shell_host(shell_host);
        }

        self.input.update(ctx, |view, ctx| {
            view.set_active_block_metadata(block_metadata.clone(), is_after_in_band_command, ctx);
            // Now that we've received the metadata for the active block, redraw the
            // prompt area so it's up to date.
            ctx.notify();
        });
    }

    pub(super) fn handle_terminal_event(
        &mut self,
        event: &ModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ModelEvent::TerminalClear => {
                self.handle_terminal_wakeup((), ctx);
                self.update_scroll_position_locking(ScrollPositionUpdate::AfterClear, ctx);
                ctx.notify();
            }
            ModelEvent::Title(title) => {
                self.terminal_title = title.to_owned();
                if self.ignore_next_set_title_event {
                    self.ignore_next_set_title_event = false;
                } else {
                    self.update_pane_configuration(ctx);
                }
            }
            ModelEvent::ClipboardStore(_, contents) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(contents.to_owned()));
            }
            ModelEvent::ClipboardLoad(_, format) => {
                self.write_to_pty(
                    format(&TerminalView::read_from_clipboard(
                        Some(self.shell_family(ctx)),
                        ctx,
                    ))
                    .into_bytes(),
                    ctx,
                );
            }
            ModelEvent::CursorBlinkingChange(_) => {}
            ModelEvent::MouseCursorDirty => {}
            ModelEvent::Bell => {
                if *TerminalSettings::as_ref(ctx).use_audible_bell {
                    if let Err(e) = AudibleBell::as_ref(ctx).ring() {
                        log::warn!("Unable to play bell: {e:#}");
                    }
                }
                // TODO(vorporeal): Remove this once we have a visual bell
                // indicator in terminal tabs.
                ctx.request_user_attention();
            }
            ModelEvent::Exit { reason } => {
                if !self.manual_pty_shutdown_requested {
                    self.maybe_send_agent_exited_shell_telemetry(ctx);
                }

                // If the pty spawn has failed, we've already inserted a banner.
                if !self.pty_spawn_failed {
                    let shell_detail = self.shell_detail.take().unwrap_or("shell".to_owned());
                    self.insert_shell_process_terminated_banner(
                        shell_terminated_banner::TerminationType::Premature {
                            shell_detail,
                            reason: *reason,
                        },
                        ctx,
                    );
                }
                // Mark the editor as disabled to ensure user interactions with
                // it are ignored.
                self.input.update(ctx, |input, ctx| {
                    input.editor().update(ctx, |editor, ctx| {
                        editor
                            .set_interaction_state(crate::editor::InteractionState::Disabled, ctx);
                    });
                });

                // If we failed to bootstrap by the time we exited, show the
                // bootstrap block so the user might be able to see what went wrong.
                if !self.is_login_shell_bootstrapped {
                    self.show_initialization_block();
                }

                if !self.pty_spawn_failed {
                    ctx.emit(Event::Exited);
                }
            }
            ModelEvent::BlockCompleted(block_completed_event) => {
                record_trace_event!("command_execution:block_completed");
                end_trace_after_next!("window:redraw:end");
                let block_completed_event_clone = block_completed_event.clone();
                self.input.update(ctx, |input, ctx| {
                    input.handle_block_completed_event(block_completed_event_clone, ctx);
                });

                // Notify find model that this block completed so it gets scanned with final output.
                let completed_block_index = block_completed_event.block_index;
                self.find_model.update(ctx, |find_model, ctx| {
                    find_model.notify_block_completed(completed_block_index, ctx);
                });

                if !matches!(block_completed_event.block_type, BlockType::BootstrapHidden) {
                    if let Some(env_var_block) = self.active_env_var_collection_block(ctx) {
                        let output_truncated =
                            if let BlockType::User(completed) = &block_completed_event.block_type {
                                Some(completed.output_truncated.clone())
                            } else {
                                None
                            };
                        env_var_block.update(ctx, move |block, ctx| {
                            if block.is_running() {
                                match output_truncated {
                                    // If we have a non-empty response we assume it's an error. We are
                                    // relying on this because we don't get a non-zero exit code for the
                                    // `export` function
                                    Some(output) if !output.is_empty() => {
                                        block.on_failed(Some(output), ctx)
                                    }
                                    _ => block.on_succeeded(ctx),
                                }
                            }
                        });
                    }
                }

                // If this block ran a possible subshell command, and it exited before the 1s timer
                // completed, abort showing the banner.
                if let Some(abort_handle) = self.warpify_state.take_subshell_banner_abort_handle() {
                    abort_handle.abort();
                }

                // In-band commands finishing should never trigger a focus change as it could steal
                // focus from the TerminalView.
                if !matches!(block_completed_event.block_type, BlockType::InBandCommand) {
                    let reset_focus = self.redetermine_terminal_focus(ctx);
                    // There are two different cases for redraws here:
                    // 1. If this terminal or its children were focused, redraw immediately after
                    //    this event.
                    // 2. Otherwise, redraw after the next terminal wakeup.
                    //
                    // Additionally, our API for measuring the latency requires installing a
                    // callback for the next redraw. We only want to install this callback in the
                    // first case because otherwise, it could be inaccurate.
                    //
                    // Since our baseline commands are all very small, when the command finishes,
                    // the same terminal almost certainly still has the focus.
                    if reset_focus {
                        if let Some(block_latency_data) = &block_completed_event.block_latency_data
                        {
                            self.install_block_latency_telemetry_callback(
                                block_latency_data.clone(),
                                ctx,
                            );
                        }
                    }
                }

                if let BlockType::User(_) = &block_completed_event.block_type {
                    self.on_user_block_completed(&block_completed_event.block_id, ctx);
                }

                // Clear any stale warpify mode so it doesn't leak into the next command's footer rendering.
                self.use_agent_footer.update(ctx, |footer, ctx| {
                    footer.clear_warpify_mode(ctx);
                });
                self.hide_use_agent_footer_in_blocklist(ctx);
                if matches!(block_completed_event.block_type, BlockType::User(_)) {
                    // Close the rich input editor if it was open (side effects
                    // like input config restore happen reactively).
                    // The auto-toggle flag is irrelevant here because the
                    // session is removed immediately afterwards.
                    self.close_cli_agent_rich_input(CLIAgentRichInputCloseReason::Other, ctx);
                    CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions_model, ctx| {
                        sessions_model.remove_session(self.view_id, ctx);
                    });
                }

                let next_block_index = block_completed_event.block_index + BlockIndex::from(1);

                // Don't populate mouse states for In-Band blocks. In-band blocks are hidden to the
                // user and there can be an arbitrarily large number of blocks as the user types
                // and interacts with the session. This in turn can cause performance and memory
                // issues since we clone the mouse states on every render.
                if !matches!(block_completed_event.block_type, BlockType::InBandCommand) {
                    self.block_list_mouse_states
                        .label_mouse_states
                        .entry(next_block_index)
                        .or_default();
                    self.block_list_mouse_states
                        .bookmark_mouse_states
                        .entry(next_block_index)
                        .or_default();
                    self.block_list_mouse_states
                        .filter_mouse_states
                        .entry(next_block_index)
                        .or_default();
                }

                // Revert the pane title to the conversation name (if any) now that
                // is_long_running() has become false. Without this, the title stays at the
                // terminal title until the shell's precmd hook fires its next SetTitle event.
                self.update_pane_configuration(ctx);
            }
            ModelEvent::VisibleBootstrapBlock => {
                // We don't want to focus the input box in the case where
                // the block list isn't bootstrapped and there's a visible
                // bootstrap block oh-my-zsh (update prompt appears). In the
                // case, we want the block to be focused because otherwise,
                // users get stuck as they'd otherwise need to click into the
                // box to respond to whether or not they want to update oh my zsh.
                self.focus_terminal(ctx);
            }
            ModelEvent::AfterBlockStarted {
                command,
                is_for_in_band_command,
                block_id,
                ..
            } => {
                let did_any_session_contains_remote_blocks =
                    self.any_session_contains_remote_blocks;
                self.any_session_contains_remote_blocks |=
                    self.active_block_is_considered_remote(ctx);
                if self.any_session_contains_remote_blocks != did_any_session_contains_remote_blocks
                {
                    self.update_focused_terminal_info(ctx);
                }

                if *is_for_in_band_command {
                    return;
                }
                self.did_notify_long_running = false;

                // Snapshot the prompt state as of when the command began executing.
                // Commands may themselves affect the prompt (if running `git checkout`), for
                // example, so we want the saved prompt state to match what the user saw when
                // they entered the command.
                let prompt_snapshot = self.current_prompt.as_ref(ctx).snapshot(ctx);
                self.model
                    .lock()
                    .block_list_mut()
                    .active_block_mut()
                    .set_prompt_snapshot(prompt_snapshot);

                // Clear any previously active AM query suggestion banners and hidden blocks.
                self.clear_prompt_suggestions(ctx);
                self.drop_hidden_passive_ai_blocks(ctx);

                // If the first word of the command is a shell alias, expand it
                // for subshell/SSH detection. This enables warpification for
                // aliased SSH commands (e.g. `alias myssh='ssh user@host'`).
                let expanded_command = self
                    .active_block_session_id()
                    .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id))
                    .and_then(|session| {
                        let (first_word, rest) = command_first_word_and_suffix(command)?;
                        let alias_value = session.alias_value(first_word)?;
                        Some(format!("{alias_value}{rest}"))
                    });
                let warpify_command = expanded_command.as_deref().unwrap_or(command.as_str());

                // Check if the current running command spawns a subshell eligible for Warpification.
                let shell_family = self.shell_family(ctx);
                let warpify_settings = WarpifySettings::as_ref(ctx);
                let is_compatible_subshell_command = warpify_settings
                    .is_compatible_subshell_command(command, shell_family)
                    || warpify_settings
                        .is_compatible_subshell_command(warpify_command, shell_family);
                let command_is_denylisted = warpify_settings
                    .is_denylisted_subshell_command(command)
                    || warpify_settings.is_denylisted_subshell_command(warpify_command);
                // Never warpify or surface warpification for agent-requested commands.
                let has_ai_metadata = self
                    .model
                    .lock()
                    .block_list()
                    .active_block()
                    .agent_interaction_metadata()
                    .is_some();

                if is_compatible_subshell_command {
                    if command_is_denylisted || has_ai_metadata {
                        // Don't auto-warpify or surface warpification for these commands.
                    } else if let Some(shell_type) = self.pending_auto_bootstrap_shell_type.take() {
                        // If there is a subshell we're waiting to bootstrap until we receive
                        // the preexec hook, now we can bootstrap it.
                        let auto_warpify_abort_handle = ctx.spawn_abortable(
                            Timer::after(Duration::from_millis(AUTO_WARPIFY_DELAY)),
                            move |me, _, ctx| {
                                me.trigger_subshell_bootstrap(Some(shell_type), false, ctx);
                            },
                            |_, _| (),
                        );
                        self.warpify_state
                            .add_auto_warpify_abort_handle(auto_warpify_abort_handle);
                    } else {
                        // Wait 1 second before showing the banner, just to make sure the
                        // command stays running for a bit. If the command fails instantly,
                        // we don't want to flicker the banner away so quickly.
                        let command = command.clone();
                        self.warpify_state
                            .add_subshell_banner_abort_handle(ctx.spawn_abortable(
                                Timer::after(*SUBSHELL_BANNER_DELAY_DURATION),
                                |view, _, ctx| {
                                    if FeatureFlag::WarpifyFooter.is_enabled() {
                                        view.show_warpify_footer(
                                            WarpificationMode::subshell(command),
                                            ctx,
                                        );
                                    } else {
                                        view.handle_action(
                                            &TerminalAction::ShowSubshellBanner(command),
                                            ctx,
                                        );
                                    }
                                },
                                |_, _| {},
                            ));
                    }
                } else {
                    if !has_ai_metadata {
                        if let Some(ssh_host) =
                            parse_interactive_ssh_command(warpify_command).map(|cmd| cmd.host)
                        {
                            if !self.model.lock().tmux_control_mode_active() {
                                self.warpify_state
                                    .set_pending_ssh_host(warpify_command.to_string(), ssh_host);
                                self.model.lock().start_notify_on_end_of_ssh_login();
                                ctx.emit(Event::TerminalViewStateChanged);
                            }
                        } else {
                            self.warpify_state.clear_pending_ssh_host();

                            ctx.spawn(
                                Timer::after(Duration::from_millis(
                                    LONG_RUNNING_COMMAND_DURATION_MS,
                                )),
                                move |me, _, ctx| {
                                    // Detect CLI agent and create session before
                                    // showing the footer, so the session drives
                                    // the footer rather than the other way around.
                                    let detection = {
                                        let model = me.model.lock();
                                        me.detect_cli_agent_from_model(&model, ctx)
                                    };
                                    let view_id = me.view_id;
                                    CLIAgentSessionsModel::handle(ctx).update(
                                        ctx,
                                        |sessions_model, ctx| match detection {
                                            Some((agent, ref custom_command_prefix))
                                                if !sessions_model
                                                    .session(view_id)
                                                    .is_some_and(|s| s.agent == agent) =>
                                            {
                                                let remote_host =
                                                    me.active_session_remote_host(ctx);
                                                sessions_model.set_session(
                                                    view_id,
                                                    CLIAgentSession {
                                                        agent,
                                                        status: CLIAgentSessionStatus::InProgress,
                                                        session_context:
                                                            CLIAgentSessionContext::default(),
                                                        input_state: CLIAgentInputState::Closed,
                                                        should_auto_toggle_input: *AISettings::as_ref(
                                                            ctx,
                                                        )
                                                        .auto_open_rich_input_on_cli_agent_start,
                                                        listener: None,
                                                        plugin_version: None,
                                                        remote_host,
                                                        draft_text: None,
                                                        custom_command_prefix: custom_command_prefix.clone(),
                                                    },
                                                    ctx,
                                                );
                                            }
                                            _ => {}
                                        },
                                    );

                                    // Codex doesn't use the sentinel-based plugin protocol,
                                    // so create the listener proactively on command detection
                                    // (rather than waiting for a SessionStart event).
                                    if matches!(detection, Some((CLIAgent::Codex, _))) {
                                        me.register_cli_agent_listener_without_session_start_event(
                                            CLIAgent::Codex,
                                            ctx,
                                        );
                                    }

                                    me.maybe_show_use_agent_footer_in_blocklist(ctx);
                                    me.maybe_auto_open_cli_agent_rich_input(ctx);
                                    me.input.update(ctx, |input, ctx| {
                                        input.universal_developer_input_button_bar().update(
                                            ctx,
                                            |bar, ctx| {
                                                bar.update_segmented_control_disabled_state(ctx);
                                            },
                                        )
                                    });
                                    // Update agent view back button state when command becomes long-running
                                    if FeatureFlag::AgentView.is_enabled()
                                        && me.agent_view_controller.as_ref(ctx).is_fullscreen()
                                    {
                                        me.update_agent_view_back_button_state(ctx);
                                        me.update_agent_view_pane_header(ctx);
                                    }
                                },
                            );
                        }
                    }

                    self.maybe_insert_setup_command_blocks(block_id, ctx);

                    self.set_current_state(TerminalViewState::LongRunning, ctx);
                    ctx.emit(Event::BlockStarted {
                        is_for_in_band_command: *is_for_in_band_command,
                    });
                }
            }
            ModelEvent::AfterBlockCompleted(AfterBlockCompletedEvent {
                command_finished_to_precmd_delay,
                block_type,
                num_secrets_obfuscated,
                cloud_workflow_id,
                cloud_env_var_collection_id,
            }) => {
                // To automatically warpify a subshell, we run the relevant command to open the
                // subshell and create a future to delay bootstrapping the subshell long enough for
                // the command to complete. We receive AfterBlockCompleted if the subshell command
                // returns an error or the user exits the subshell. Here we abort the future to
                // avoid an attempt to trigger bootstrapping if the subshell command failed. If the
                // future already resolved, abort has no effect. We handle this as early as possible
                // because the abort is time sensitive.
                self.warpify_state.abort_auto_warpify();

                let active_session = self
                    .active_block_session_id()
                    .and_then(|id| self.sessions.as_ref(ctx).get(id));
                if let Some(active_session) = active_session {
                    if !active_session.has_attempted_to_load_external_commands() {
                        ctx.background_executor()
                            .spawn(async move { active_session.load_external_commands().await })
                            .detach();
                    }
                }

                if let Some(delay) = command_finished_to_precmd_delay {
                    let delay_ms = delay.as_millis() as u64;
                    let honor_ps1_enabled = match &block_type {
                        // If we have access to the value of honor_ps1 that the
                        // block was holding, use that.
                        BlockType::User(UserBlockCompleted {
                            serialized_block, ..
                        })
                        | BlockType::BootstrapVisible(serialized_block) => {
                            serialized_block.honor_ps1
                        }
                        // Otherwise, grab the current value.
                        _ => *SessionSettings::as_ref(ctx).honor_ps1,
                    };
                    if let BlockType::User(user_block_completed) = block_type {
                        let is_universal_developer_input_enabled =
                            InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx);
                        let is_in_agent_view = self.agent_view_controller.as_ref(ctx).is_active();
                        send_telemetry_from_ctx!(
                            TelemetryEvent::BlockCompleted {
                                block_finished_to_precmd_delay_ms: delay_ms,
                                honor_ps1_enabled,
                                num_secrets_redacted: *num_secrets_obfuscated,
                                num_output_lines: user_block_completed.num_output_lines,
                                num_output_lines_truncated: user_block_completed
                                    .num_output_lines_truncated,
                                terminal_session_id: user_block_completed
                                    .serialized_block
                                    .session_id,
                                is_udi_enabled: is_universal_developer_input_enabled,
                                is_in_agent_view,
                            },
                            ctx
                        );

                        // On dogfood only, we're interested in the block commands, durations,
                        // and exit codes to trial Warp Analytics.
                        if ChannelState::channel().is_dogfood() {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::BlockCompletedOnDogfoodOnly {
                                    block_finished_to_precmd_delay_ms: delay_ms,
                                    honor_ps1_enabled,
                                    num_secrets_redacted: *num_secrets_obfuscated,
                                    num_output_lines: user_block_completed.num_output_lines,
                                    num_output_lines_truncated: user_block_completed
                                        .num_output_lines_truncated,
                                    command: user_block_completed.command.clone(),
                                    duration: self
                                        .block_duration(&user_block_completed.serialized_block)
                                        .unwrap_or_default(),
                                    exit_code: user_block_completed.serialized_block.exit_code,
                                    terminal_session_id: user_block_completed
                                        .serialized_block
                                        .session_id,
                                },
                                ctx
                            );
                        }
                    }
                }
                let active_session_id = self.active_block_session_id();
                if let Some(block_id) = self
                    .warpify_state
                    .get_completed_warpify_session_id(active_session_id, ctx)
                {
                    self.remove_ssh_block_by_id(block_id);
                }

                self.dismiss_warpify_banner(
                    &RememberForWarpification::DoNotRememberSubshellCommand,
                    ctx,
                );

                let pending_command_succeeded = match &block_type {
                    BlockType::User(UserBlockCompleted {
                        serialized_block, ..
                    }) => Some(serialized_block.exit_code.was_successful()),
                    BlockType::BootstrapHidden
                    | BlockType::BootstrapVisible(_)
                    | BlockType::Restored
                    | BlockType::InBandCommand
                    | BlockType::Background(_)
                    | BlockType::Static => None,
                };

                // Emit PendingCommandCompleted when a pending command's block
                // finishes (e.g. tab config setup commands like `git worktree add`).
                if self.awaiting_pending_command_completion {
                    if let Some(command_succeeded) = pending_command_succeeded {
                        self.awaiting_pending_command_completion = false;
                        if command_succeeded && self.set_next_pending_command_from_queue(ctx) {
                            // The delayed pending-command scheduler below will
                            // submit the next queued command as a separate block.
                        } else {
                            if !command_succeeded {
                                self.pending_command_queue.clear();
                            }
                            ctx.emit(Event::PendingCommandCompleted);

                            // If agent view entry was deferred until setup commands
                            // finished, enter it now (unless suppressed by onboarding).
                            if self.enter_agent_view_after_pending_commands {
                                self.enter_agent_view_after_pending_commands = false;
                                self.enter_agent_view_for_new_conversation(
                                    None,
                                    AgentViewEntryOrigin::Input {
                                        was_prompt_autodetected: false,
                                    },
                                    ctx,
                                );
                            }
                        }
                    }
                }

                // For the case when the user uses session configuration with a
                // command list, we execute the command after a delay.
                // The delay is necessary because the shell needs a tiny bit of
                // extra time after the last precmd function is finished.
                // Additionally, it's possible for hooks to install themselves after the warp
                // precmd. For example, `fig_precmd` does this.
                if self.is_login_shell_bootstrapped {
                    let _ = ctx.spawn(
                        async move {
                            warpui::r#async::Timer::after(EXECUTE_PENDING_COMMAND_DELAY).await;
                        },
                        Self::execute_pending_command,
                    );
                }

                // When a block completes, we need to update the prompt for the next
                // active block. We specifically want to avoid doing this for in-band
                // commands because otherwise we'll create a loop if updating the prompt
                // involves running an in-band command. Similarly, we want to ensure
                // that the shell has been bootstrapped. Since we're in the BlockCompleted
                // event, that also implies we would have received the first precmd
                // so we know that the active block has valid metadata.
                if !matches!(block_type, BlockType::InBandCommand)
                    && self
                        .model
                        .lock()
                        .block_list()
                        .is_bootstrapping_precmd_done()
                {
                    self.refresh_warp_prompt(ctx);

                    // If the completed command was a `gh` or `gt` invocation, eagerly refresh PR
                    // info since these don't touch .git/ and won't be caught by the filesystem watcher.
                    #[cfg(feature = "local_fs")]
                    if (FeatureFlag::GitOperationsInCodeReview.is_enabled()
                        || FeatureFlag::GithubPrPromptChip.is_enabled())
                        && match &block_type {
                            BlockType::User(user_block_completed) => {
                                let command = user_block_completed.command.as_str();
                                let top_level = user_block_completed
                                    .serialized_block
                                    .session_id
                                    .and_then(|session_id| {
                                        self.sessions.as_ref(ctx).get(session_id)
                                    })
                                    .and_then(|session| {
                                        let escape_char = session.shell_family().escape_char();
                                        let cmd =
                                            warp_completer::parsers::simple::top_level_command(
                                                command,
                                                escape_char,
                                            )?;
                                        let cmd = session
                                            .alias_value(cmd.as_str())
                                            .and_then(|alias| {
                                                warp_completer::parsers::simple::top_level_command(
                                                    alias,
                                                    escape_char,
                                                )
                                            })
                                            .unwrap_or(cmd);
                                        Some(cmd)
                                    })
                                    .or_else(|| {
                                        command.split_whitespace().next().map(|cmd| cmd.to_owned())
                                    });

                                matches!(top_level.as_deref(), Some("gh" | "gt"))
                            }
                            _ => false,
                        }
                    {
                        self.refresh_pr_info_after_gh_or_gt_command(ctx);
                    }
                }

                if let BlockType::User(block_completed) = block_type {
                    if let Some(block_duration) =
                        self.block_duration(&block_completed.serialized_block)
                    {
                        self.maybe_send_block_completed_notification(
                            block_completed,
                            block_duration,
                            ctx,
                        );
                    }

                    // We don't want any suggestion UIs on AI requested blocks.
                    if !block_completed.was_part_of_agent_interaction {
                        self.maybe_generate_command_suggestions(block_completed, ctx);

                        if self.can_suggest_alias_expansion(ctx) {
                            self.maybe_suggest_alias_expansion(block_completed, ctx);
                        }

                        self.maybe_suggest_open_in_warp(block_completed, ctx);
                    }

                    // Check if the user tried to run an AWS login command but AWS CLI wasn't installed.
                    // This runs after other suggestion checks and may add its own banner alongside them.
                    self.maybe_show_aws_cli_not_installed_suggestion(
                        block_completed.serialized_block.exit_code,
                        ctx,
                    );

                    // Check for environment creation command completion during /init flow
                    if block_completed.was_part_of_agent_interaction
                        && self.has_active_init_project(ctx)
                    {
                        self.maybe_handle_environment_create_command(block_completed, ctx);
                    }

                    let terminal_view_state = {
                        let model = self.model.lock();
                        match model.block_list().last_non_hidden_block() {
                            Some(block) if block.has_failed() => TerminalViewState::Errored,
                            _ => TerminalViewState::Normal,
                        }
                    };
                    self.did_notify_long_running = false;
                    self.set_current_state(terminal_view_state, ctx);

                    // Update agent view back button state when command completes
                    if FeatureFlag::AgentView.is_enabled()
                        && self.agent_view_controller.as_ref(ctx).is_fullscreen()
                    {
                        self.update_agent_view_back_button_state(ctx);
                        self.update_agent_view_pane_header(ctx);
                    }

                    let exit_code_data =
                        &json!({"exit_code": block_completed.serialized_block.as_ref().exit_code})
                            .to_string();

                    // If the block was a cloud workflow, record the workflow execution as an object action.
                    if let Some(cloud_workflow_id) = cloud_workflow_id {
                        let id_and_type = CloudObjectTypeAndId::Workflow(*cloud_workflow_id);
                        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                            update_manager.record_object_action(
                                id_and_type,
                                ObjectActionType::Execute,
                                Some(exit_code_data.clone()),
                                ctx,
                            )
                        });
                    }

                    if let Some(cloud_env_var_collection_id) = cloud_env_var_collection_id {
                        let id_and_type = CloudObjectTypeAndId::GenericStringObject {
                            object_type: GenericStringObjectFormat::Json(
                                JsonObjectType::EnvVarCollection,
                            ),

                            id: *cloud_env_var_collection_id,
                        };
                        UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                            update_manager.record_object_action(
                                id_and_type,
                                ObjectActionType::Execute,
                                Some(exit_code_data.clone()),
                                ctx,
                            )
                        });
                    }

                    if let (
                        Some(active_session_id),
                        exit_code,
                        Some(start_ts),
                        Some(completed_ts),
                        Some(model_event_sender),
                    ) = (
                        self.active_block_session_id(),
                        block_completed.serialized_block.as_ref().exit_code,
                        block_completed.serialized_block.as_ref().start_ts,
                        block_completed.serialized_block.as_ref().completed_ts,
                        &self.model_event_sender,
                    ) {
                        History::handle(ctx).update(ctx, move |history, _ctx| {
                            history.mark_command_as_finished(
                                active_session_id,
                                start_ts,
                                completed_ts,
                                exit_code,
                            );
                        });

                        let sender_clone = model_event_sender.clone();
                        let update_finished_command_event =
                            persistence::ModelEvent::UpdateFinishedCommand {
                                metadata: FinishedCommandMetadata {
                                    exit_code,
                                    start_ts,
                                    completed_ts,
                                    session_id: active_session_id,
                                },
                            };
                        let _ = ctx.spawn(
                            async move {
                                // Sending over a sync sender can block the current thread, so we do this async.
                                sender_clone.send(update_finished_command_event)
                            },
                            move |_, res, _| {
                                if let Err(err) = res {
                                    log::error!(
                                        "Error sending UpdateFinishedCommand event: {err:?}"
                                    );
                                }
                            },
                        );
                    }

                    #[cfg(not(target_family = "wasm"))]
                    crate::system::SystemInfo::handle(ctx).update(ctx, |system_info, _ctx| {
                        system_info.handle_block_created();
                    });

                    // Emit the event to the parent view. This will save the block to sqlite if
                    // session restoration is enabled.
                    ctx.emit(Event::BlockCompleted {
                        block: block_completed.serialized_block.clone(),
                        is_local: !self.is_block_considered_remote(
                            block_completed.serialized_block.session_id,
                            Some(&block_completed.command),
                            ctx,
                        ),
                    });
                } else if let BlockType::Background(serialized_block) = block_type {
                    // Because background output blocks are before the active block, they need to be saved
                    // via a BlockCompleted event but don't affect focus or input.
                    ctx.emit(Event::BlockCompleted {
                        block: serialized_block.clone(),
                        is_local: !self.is_block_considered_remote(
                            serialized_block.session_id,
                            None,
                            ctx,
                        ),
                    });
                } else if let BlockType::BootstrapVisible(serialized_block) = block_type {
                    // Re-compute the focus after the visible bootstrap block has completed.
                    self.redetermine_terminal_focus(ctx);
                    ctx.emit(Event::BlockCompleted {
                        block: serialized_block.clone(),
                        is_local: !self.is_block_considered_remote(
                            serialized_block.session_id,
                            None,
                            ctx,
                        ),
                    });
                }

                self.input.update(ctx, |input, ctx| {
                    input.handle_after_block_completed_event(block_type.clone(), ctx);
                });
            }
            ModelEvent::BackgroundBlockStarted => {
                // For now, this event is only used for telemetry. It may also
                // be useful to request attention if the user's session starts
                //receiving background output, or to auto-scroll it.
                send_telemetry_from_ctx!(TelemetryEvent::BackgroundBlockStarted, ctx);
            }
            ModelEvent::PreInteractiveSSHSession => {}
            ModelEvent::SSH(remote_shell) => {
                if let Some(shell) = ShellType::from_name(remote_shell) {
                    if shell.is_fully_supported_remotely() {
                        // Start a bootstrap timer for the SSH session, so we can log when the session
                        // takes too long to initialize
                        self.start_bootstrap_timer(BOOTSTRAP_FAILED_DURATION, ctx);
                    }
                }
                send_telemetry_from_ctx!(
                    TelemetryEvent::SSHBootstrapAttempt(remote_shell.clone()),
                    ctx
                );
            }
            ModelEvent::SSHControlMasterError => {
                self.handle_control_master_error(ctx);
            }
            ModelEvent::BlockMetadataReceived(block_metadata_received_event) => {
                self.apply_block_metadata_update(
                    &block_metadata_received_event.block_metadata,
                    block_metadata_received_event.is_after_in_band_command,
                    block_metadata_received_event.is_done_bootstrapping,
                    BlockMetadataUpdateSource::Precmd,
                    ctx,
                );
            }
            ModelEvent::BlockWorkingDirectoryUpdated(block_working_directory_updated_event) => {
                self.apply_block_metadata_update(
                    &block_working_directory_updated_event.block_metadata,
                    block_working_directory_updated_event.is_for_in_band_command,
                    block_working_directory_updated_event.is_done_bootstrapping,
                    BlockMetadataUpdateSource::Osc7,
                    ctx,
                );
                // Recompute Warp-prompt chip values (notably the
                // `WorkingDirectory` chip text that feeds the vertical-tab
                // subtitle via `display_working_directory`). The chip
                // generator reads from `CurrentPrompt::latest_context`, which
                // is only refreshed through `refresh_warp_prompt` →
                // `current_prompt.update_context`. In the normal precmd flow
                // that refresh is triggered by `BlockCompleted`, but an OSC 7
                // fires mid-command — the block never completes — so without
                // this call the chip text stays stuck on the previous CWD
                // even though the underlying block metadata is up to date.
                //
                // Skip in-band-command blocks for the same reason
                // `apply_block_metadata_update` bails early on them: in-band
                // commands don't change CWD, and refreshing the prompt here
                // can re-fire chip generators that schedule another in-band
                // command, leading to a refresh loop.
                if !block_working_directory_updated_event.is_for_in_band_command {
                    self.refresh_warp_prompt(ctx);
                }
            }

            ModelEvent::TerminalModeSwapped(mode) => {
                #[cfg(feature = "local_tty")]
                {
                    let active_command = self
                        .model
                        .lock()
                        .block_list()
                        .active_block()
                        .top_level_command(self.sessions.as_ref(ctx));
                    // If we don't know what the top-level command is,
                    // we should still perform the redundant resize.
                    if active_command.is_none_or(|cmd| {
                        !ALT_SCREEN_APPS_THAT_MUST_MATCH_BLOCKLIST_PADDING.contains(cmd.as_str())
                    }) {
                        // Since the alt-screen and blocklist have different sizes,
                        // let's make sure to refresh the winsize when switching
                        // back and forth between these modes.
                        self.refresh_size(ctx);

                        if matches!(mode, TerminalMode::AltScreen)
                            && matches!(
                                *TerminalSettings::as_ref(ctx).alt_screen_padding,
                                AltScreenPaddingMode::Custom { .. }
                            )
                        {
                            // Redundantly send resizes in case the alt-screens
                            // resize handler was not registered in time.
                            self.resize_alt_screen_redundantly(ctx);
                        }
                    }
                }

                let existing_find_options = match mode {
                    TerminalMode::AltScreen => self
                        .find_model
                        .as_ref(ctx)
                        .block_list_find_run()
                        .map(|run| run.options()),
                    TerminalMode::BlockList => self
                        .find_model
                        .as_ref(ctx)
                        .alt_screen_find_run()
                        .map(|run| run.options()),
                }
                .cloned();
                if let Some(FindOptions {
                    query: Some(query),
                    is_regex_enabled,
                    is_case_sensitive,
                    ..
                }) = existing_find_options
                {
                    // If there was an active find in the old mode, preserve and re-run the same
                    // query in the new mode.
                    self.find_model.update(ctx, |find_model, ctx| {
                        find_model.run_find(
                            FindOptions {
                                query: Some(query),
                                is_regex_enabled,
                                is_case_sensitive,
                                ..Default::default()
                            },
                            ctx,
                        );
                    });
                }

                self.input.update(ctx, |_, ctx| {
                    ctx.emit(InputEvent::InputStateChanged(match mode {
                        TerminalMode::AltScreen => InputState::Disabled,
                        TerminalMode::BlockList => InputState::Enabled,
                    }));
                });

                // Close the find bar across the screen transition.
                // We don't want to change focus unnecessarily, e.g. when
                // using synced inputs and exiting `vim`.
                if self.find_model.as_ref(ctx).is_find_bar_open() {
                    self.close_find_bar(ctx);
                    self.redetermine_global_focus(ctx);
                }

                // Update agent view back button state when alt screen becomes active/inactive
                if FeatureFlag::AgentView.is_enabled()
                    && self.agent_view_controller.as_ref(ctx).is_fullscreen()
                {
                    self.update_agent_view_back_button_state(ctx);
                }
            }
            ModelEvent::TmuxControlModeReady { .. } => {
                self.trigger_subshell_bootstrap(None, false, ctx);
            }
            ModelEvent::DetectedEndOfSshLogin(check_type) => {
                self.handle_detected_end_of_ssh_login(check_type, ctx);
            }
            ModelEvent::RemoteWarpificationIsUnavailable(reason) => {
                self.handle_remote_warpification_is_unavailable(reason.clone(), ctx);
            }
            ModelEvent::SshTmuxInstaller(tmux_installation) => {
                self.warpify_state
                    .set_tmux_installation_state(*tmux_installation);
            }
            ModelEvent::TmuxInstallFailed { line, command } => {
                let system_details = self
                    .warpify_state
                    .ssh_block_state()
                    .and_then(|s| s.get_system_details(ctx));
                self.warpify_state.abort_ssh_warpify_timeout();
                self.add_ssh_error_block(
                    WarpificationUnavailableReason::TmuxInstallFailed {
                        system_details,
                        line: Some(line.to_string()),
                        command: Some(command.to_string()),
                    },
                    ctx,
                );
            }
            ModelEvent::ExecutedInBandCommand(event) => {
                // TODO(vorporeal): Figure out a way to not need the terminal view involved
                // in this flow.
                let active_session_id = self.active_block_session_id();
                if let Some(active_session_id) = active_session_id {
                    self.sessions.update(ctx, |sessions, _ctx| {
                        sessions.handle_executed_command_event(active_session_id, event.clone());
                    });
                }
            }
            ModelEvent::InitSubshell(event) => {
                let shell_type = event.shell_type;
                self.trigger_subshell_bootstrap(Some(shell_type), false, ctx);
            }
            ModelEvent::InitSsh(event) => {
                let shell_type = event.shell_type;
                let uname = event.uname.as_ref().unwrap_or(&String::default()).clone();
                self.continue_warpify_ssh_session(&uname, shell_type, ctx);
            }
            ModelEvent::SourcedRcFileInSubshell(event) => {
                send_telemetry_from_ctx!(TelemetryEvent::ReceivedSubshellRcFileDcs, ctx);
                let shell_type = event.shell_type;
                let uname = event.uname.clone();
                let disable_tmux = event.tmux == Some(false);

                ctx.spawn(
                    async {
                        warpui::r#async::Timer::after(*TRIGGER_RC_FILE_SUBSHELL_BOOTSTRAP_DELAY)
                            .await
                    },
                    move |me, _, ctx| {
                        let uname = uname.to_owned().unwrap_or_default();
                        let (is_ssh, is_tmux_control_mode_active, has_ai_metadata) = {
                            let lock = me.model.lock();
                            let has_ai_metadata = lock
                                .block_list()
                                .active_block()
                                .agent_interaction_metadata()
                                .is_some();
                            (
                                lock.is_ssh_block(),
                                lock.tmux_control_mode_active(),
                                has_ai_metadata,
                            )
                        };
                        // Never warpify for agent-requested commands.
                        if has_ai_metadata {
                            return;
                        }
                        // To simplify the implementation, we do not support warpifying while SSH-warpified.
                        if is_tmux_control_mode_active {
                            return;
                        }
                        if is_ssh && !disable_tmux {
                            me.continue_warpify_ssh_session(&uname, shell_type, ctx);
                        } else {
                            me.trigger_subshell_bootstrap(Some(shell_type), true, ctx);
                        }
                    },
                );
            }
            ModelEvent::PromptUpdated => {
                self.input.update(ctx, |input, ctx| {
                    input.notify_and_notify_children(ctx);
                });
            }
            ModelEvent::HonorPS1OutOfSync => {}
            ModelEvent::Typeahead => {
                self.handle_typeahead_event(ctx);
            }
            ModelEvent::Handler(AnsiHandlerEvent::InitShell {
                pending_session_info,
            }) => {
                // The remote confirmed a subshell bootstrap is starting. Hide the
                // original long-running block now so the user doesn't see the
                // bootstrap payload echoed into it.
                if pending_session_info.subshell_info.is_some() {
                    let show_debug_block = BlockVisibilitySettings::as_ref(ctx)
                        .should_show_ssh_block
                        .value();
                    if !show_debug_block {
                        self.update_long_running_ssh_block_with_lock(|block| block.hide());
                    }
                }
            }
            ModelEvent::Handler(_) => {}
            ModelEvent::FinishUpdate(data) => {
                let AutoupdateStage::UpdateReady {
                    update_id: expected_update_id,
                    ..
                } = get_update_state(ctx)
                else {
                    log::warn!(
                        "Got a FinishUpdate event without AutoupdateState being UpdateReady!"
                    );
                    return;
                };
                if expected_update_id == data.update_id {
                    // Terminate this shell session so that it doesn't come
                    // back when we restore sessions after the relaunch.
                    self.shutdown_pty(ctx);
                    autoupdate::initiate_relaunch_for_update(ctx);
                } else {
                    log::warn!("Got a FinishUpdate event with non-matching update id!");
                }
            }
            ModelEvent::SelectedTextChanged => {
                ctx.emit(Event::SelectedTextChanged);
            }
            ModelEvent::ShellSpawned(shell_type) => {
                ctx.emit(Event::ShellSpawned(*shell_type));
                ctx.notify();
            }
            ModelEvent::CompletionsFinished(_data) => {}
            ModelEvent::SendCompletionsPrompt => {}
            ModelEvent::ImageReceived {
                image_id,
                image_data,
                image_protocol,
            } => {
                AssetCache::handle(ctx).update(ctx, |asset_cache, ctx| {
                    asset_cache.insert_raw_asset_bytes::<ImageType>(
                        image_id.to_string(),
                        &image_data[..],
                        ctx,
                    );
                });
                ctx.notify();
                send_telemetry_from_ctx!(
                    TelemetryEvent::ImageReceived {
                        image_protocol: *image_protocol
                    },
                    ctx
                );
            }
            ModelEvent::BootstrapPrecmdDone => {
                self.execute_pending_command((), ctx);
            }
            ModelEvent::AgentTaggedInChanged { is_tagged_in } => {
                let state = if *is_tagged_in {
                    LongRunningCommandAgentInteractionState::TaggedIn
                } else {
                    LongRunningCommandAgentInteractionState::NotInteracting
                };
                ctx.emit(Event::LongRunningCommandAgentInteractionStateChanged { state });
            }
            ModelEvent::PluggableNotification { title, body } => {
                // Intercept structured CLI agent notifications (e.g. from Claude Code plugin).
                // The listener's own subscription handles subsequent events; we just
                // suppress the raw JSON from becoming a toast/desktop notification.
                if title.as_deref() == Some(CLI_AGENT_NOTIFICATION_SENTINEL) {
                    self.handle_cli_agent_notification(title.as_deref(), body, ctx);
                    return;
                }

                // Suppress OSC 9 notifications when a Codex listener is active.
                // The listener's subscription handles these via CodexSessionHandler.
                if title.is_none() {
                    let has_codex_listener = CLIAgentSessionsModel::as_ref(ctx)
                        .session(self.view_id)
                        .is_some_and(|s| s.agent == CLIAgent::Codex && s.listener.is_some());
                    if has_codex_listener {
                        return;
                    }
                }

                if self.is_navigated_away_from_window(ctx) {
                    let notification_title =
                        title.clone().unwrap_or_else(|| "Notification".to_string());
                    let notification = BlockNotification {
                        title: notification_title,
                        body: body.clone(),
                    };
                    ctx.emit(Event::SendNotification(notification));
                } else {
                    ctx.emit(Event::PluggableNotification {
                        title: title.clone(),
                        body: body.clone(),
                    });
                }
            }
            ModelEvent::ExitShell { session_id } => {
                // Drop the remote server client for this session before the
                // user's outer ssh tunnel starts closing. The last
                // `Arc<RemoteServerClient>` carries an owned `Child` for the
                // `ssh … remote-server-proxy` subprocess; dropping it kills
                // that child via `kill_on_drop`, which closes the
                // multiplexed channel on the ControlMaster so the foreground
                // ssh can exit cleanly instead of hanging.
                #[cfg(not(target_family = "wasm"))]
                if FeatureFlag::SshRemoteServer.is_enabled() {
                    use crate::remote_server::manager::RemoteServerManager;
                    RemoteServerManager::handle(ctx).update(
                        ctx,
                        |mgr: &mut RemoteServerManager, ctx| {
                            mgr.deregister_session(*session_id, ctx);
                        },
                    );
                }
                // The remote-server manager only exists on non-wasm targets,
                // so this handler is a no-op on wasm.
                #[cfg(target_family = "wasm")]
                let _ = session_id;
            }
            // Handled by RemoteServerController via model subscription.
            ModelEvent::SshInitShell { .. } => {}
            ModelEvent::RemoteServerBlockRequested { session_id } => {
                self.show_ssh_remote_server_choice_block(*session_id, ctx);
            }
        }
    }

    /// Creates the [`SshRemoteServerChoiceView`] and inserts it as a
    /// rich content block pinned to the bottom of the block list.
    fn show_ssh_remote_server_choice_block(
        &mut self,
        session_id: SessionId,
        ctx: &mut ViewContext<Self>,
    ) {
        let already_present = self.rich_content_views.iter().any(|view| {
            matches!(
                view.metadata(),
                Some(RichContentMetadata::SshRemoteServerChoiceBlock { handle })
                if handle.as_ref(ctx).session_id() == session_id
            )
        });
        if already_present {
            return;
        }

        let choice_view =
            ctx.add_typed_action_view(|ctx| SshRemoteServerChoiceView::new(session_id, ctx));

        ctx.subscribe_to_view(&choice_view, move |me, _, event, ctx| match event {
            SshRemoteServerChoiceViewEvent::Install => {
                me.remove_ssh_remote_server_choice_block(session_id, ctx);
                ctx.emit(Event::RemoteServerInstallRequested { session_id });
            }
            SshRemoteServerChoiceViewEvent::Skip => {
                me.remove_ssh_remote_server_choice_block(session_id, ctx);
                ctx.emit(Event::RemoteServerSkipRequested { session_id });
            }
            SshRemoteServerChoiceViewEvent::OpenWarpifySettings => {
                ctx.emit(Event::OpenSettings(SettingsSection::Warpify));
            }
        });

        self.insert_rich_content(
            None,
            choice_view.clone(),
            Some(RichContentMetadata::SshRemoteServerChoiceBlock {
                handle: choice_view,
            }),
            RichContentInsertionPosition::PinToBottom,
            ctx,
        );

        self.redetermine_global_focus(ctx);
    }

    /// Returns a clone of the `SshRemoteServerChoiceView` handle for the
    /// first active SSH remote-server choice block, if any.
    pub(super) fn active_ssh_remote_server_choice_block(
        &self,
    ) -> Option<ViewHandle<SshRemoteServerChoiceView>> {
        self.rich_content_views.iter().find_map(|view| {
            if let Some(RichContentMetadata::SshRemoteServerChoiceBlock { handle }) =
                view.metadata()
            {
                Some(handle.clone())
            } else {
                None
            }
        })
    }

    /// Returns `true` when the pending session has a connecting remote-server setup state
    /// and no failure banner is already shown for that session.
    pub(super) fn show_remote_server_loading_footer(
        &self,
        model: &TerminalModel,
        app: &AppContext,
    ) -> bool {
        if !FeatureFlag::SshRemoteServer.is_enabled() {
            return false;
        }
        // Don't show the loading footer while the choice block is visible;
        // the choice block replaces it.
        if self.active_ssh_remote_server_choice_block().is_some() {
            return false;
        }
        let Some(pending_sid) = model.pending_session_id() else {
            return false;
        };
        let has_failed_banner = self.rich_content_views.iter().any(|view| {
            matches!(
                view.metadata(),
                Some(RichContentMetadata::SshRemoteServerFailedBanner { handle })
                if handle.as_ref(app).session_id() == pending_sid
            )
        });
        if has_failed_banner {
            return false;
        }
        self.sessions
            .as_ref(app)
            .remote_server_setup_state(pending_sid)
            .is_some_and(|state| state.is_in_progress())
    }

    /// Renders a shimmering loading footer in place of the input editor
    /// while the remote server is being installed or initialized.
    pub(super) fn render_remote_server_loading_footer(
        &self,
        model: &TerminalModel,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let message = model
            .pending_session_id()
            .and_then(|sid| {
                self.sessions
                    .as_ref(app)
                    .remote_server_setup_state(sid)
                    .map(|state| match state {
                        RemoteServerSetupState::Checking => "Checking...".to_string(),
                        RemoteServerSetupState::Installing {
                            progress_percent: Some(p),
                        } => format!("Installing... ({p}%)"),
                        RemoteServerSetupState::Installing {
                            progress_percent: None,
                        } => "Installing...".to_string(),
                        RemoteServerSetupState::Updating => "Updating...".to_string(),
                        RemoteServerSetupState::Initializing => "Initializing...".to_string(),
                        _ => "Starting shell...".to_string(),
                    })
            })
            .unwrap_or_else(|| "Starting shell...".to_string());

        let shimmer_element = shimmering_warp_loading_text(
            message,
            appearance.monospace_font_size() - 2.,
            self.remote_server_shimmer_handle.clone(),
            app,
        );

        Container::new(shimmer_element)
            .with_padding_left(*PADDING_LEFT)
            .with_vertical_padding(8.)
            .finish()
    }

    /// Creates and inserts the install-failed banner as rich content.
    pub(super) fn show_ssh_remote_server_failed_banner(
        &mut self,
        session_id: SessionId,
        error: remote_server::transport::UserFacingError,
        ctx: &mut ViewContext<Self>,
    ) {
        let already_present = self.rich_content_views.iter().any(|view| {
            matches!(
                view.metadata(),
                Some(RichContentMetadata::SshRemoteServerFailedBanner { handle })
                if handle.as_ref(ctx).session_id() == session_id
            )
        });
        if already_present {
            return;
        }

        let banner =
            ctx.add_typed_action_view(|_| SshRemoteServerFailedBanner::new(session_id, error));

        ctx.subscribe_to_view(&banner, move |me, _, event, ctx| match event {
            SshRemoteServerFailedBannerEvent::Dismissed => {
                me.remove_ssh_remote_server_failed_banner(session_id, ctx);
            }
        });

        self.insert_rich_content(
            None,
            banner.clone(),
            Some(RichContentMetadata::SshRemoteServerFailedBanner { handle: banner }),
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: true,
            },
            ctx,
        );
    }

    /// Removes any install-failed banner for the given session.
    pub(super) fn remove_ssh_remote_server_failed_banner(
        &mut self,
        session_id: SessionId,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut view_ids_to_remove = Vec::new();
        for rich_content in self.rich_content_views.iter() {
            if let Some(RichContentMetadata::SshRemoteServerFailedBanner { handle }) =
                rich_content.metadata()
            {
                if handle.as_ref(ctx).session_id() == session_id {
                    view_ids_to_remove.push(rich_content.view_id());
                }
            }
        }

        if view_ids_to_remove.is_empty() {
            return;
        }

        let mut model = self.model.lock();
        for view_id in &view_ids_to_remove {
            model.block_list_mut().remove_rich_content(*view_id);
        }
        drop(model);
        self.rich_content_views
            .retain(|rich_content| !view_ids_to_remove.contains(&rich_content.view_id()));
        ctx.notify();
    }

    /// Removes [`SshRemoteServerChoiceView`] with the given `session_id`, if present.
    pub(super) fn remove_ssh_remote_server_choice_block(
        &mut self,
        session_id: SessionId,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut view_ids_to_remove = Vec::new();
        for rich_content in self.rich_content_views.iter() {
            if let Some(RichContentMetadata::SshRemoteServerChoiceBlock { handle }) =
                rich_content.metadata()
            {
                if handle.as_ref(ctx).session_id() == session_id {
                    view_ids_to_remove.push(rich_content.view_id());
                }
            }
        }

        if view_ids_to_remove.is_empty() {
            return;
        }

        let mut model = self.model.lock();
        for view_id in &view_ids_to_remove {
            model.block_list_mut().remove_rich_content(*view_id);
        }
        drop(model);
        self.rich_content_views
            .retain(|rich_content| !view_ids_to_remove.contains(&rich_content.view_id()));
        ctx.notify();
    }

    /// Handles an OSC 777 event with the `warp://cli-agent` sentinel title.
    /// On `session_start`, creates a `CLIAgentSessionListener` that subscribes
    /// to subsequent events from this terminal's PTY.
    pub(super) fn handle_cli_agent_notification(
        &mut self,
        title: Option<&str>,
        body: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(notification) = parse_event(title, body) else {
            return;
        };

        if !is_agent_supported(&notification.agent) {
            return;
        }
        if !self.register_cli_agent_listener_from_event(&notification, ctx) {
            return;
        }

        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions_model, ctx| {
            sessions_model.update_from_event(self.view_id, &notification, ctx);
        });

        if notification.event == CLIAgentEventType::SessionStart {
            send_telemetry_from_ctx!(
                TelemetryEvent::CLIAgentPluginDetected {
                    cli_agent: notification.agent.into(),
                },
                ctx
            );
            self.maybe_auto_open_cli_agent_rich_input(ctx);
        }
    }

    pub(super) fn register_cli_agent_listener_from_event(
        &mut self,
        notification: &CLIAgentEvent,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if !is_agent_supported(&notification.agent) {
            return false;
        }
        let has_listener = CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .is_some_and(|s| s.listener.is_some());
        if has_listener {
            return false;
        }

        let model_events_handle = self.model_events_handle.clone();
        let view_id = self.view_id;
        let agent = notification.agent;
        let listener = ctx.add_model(|ctx| {
            CLIAgentSessionListener::new(view_id, agent, &model_events_handle, ctx)
        });
        let remote_host = self.active_session_remote_host(ctx);
        let should_auto_toggle_input =
            *AISettings::as_ref(ctx).auto_open_rich_input_on_cli_agent_start;
        // Seed context from the event that caused registration before the
        // listener subscribes to future events.
        CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions_model, ctx| {
            sessions_model.register_listener(
                view_id,
                agent,
                notification.cwd.clone(),
                notification.project.clone(),
                notification.session_id.clone(),
                notification.payload.plugin_version.clone(),
                remote_host,
                should_auto_toggle_input,
                listener,
                ctx,
            );
        });
        true
    }

    /// Creates and registers a listener for flows without a `SessionStart` event.
    pub(super) fn register_cli_agent_listener_without_session_start_event(
        &mut self,
        agent: CLIAgent,
        ctx: &mut ViewContext<Self>,
    ) {
        // No SessionStart event in this path (mid-session install/update).
        // Assume the just-installed plugin meets the minimum version for this agent
        // so the update chip doesn't flash before the user runs /reload-plugins.
        #[cfg(not(target_family = "wasm"))]
        let plugin_version =
            plugin_manager_for(agent).map(|m| m.minimum_plugin_version().to_owned());
        #[cfg(target_family = "wasm")]
        let plugin_version = None;
        let notification = CLIAgentEvent {
            v: 1,
            agent,
            event: CLIAgentEventType::SessionStart,
            session_id: None,
            cwd: None,
            project: None,
            payload: CLIAgentEventPayload {
                plugin_version,
                ..Default::default()
            },
        };
        if self.register_cli_agent_listener_from_event(&notification, ctx) {
            self.maybe_auto_open_cli_agent_rich_input(ctx);
        }
    }

    fn child_conversation_id_for_cli_status_updates(
        &self,
        ctx: &AppContext,
    ) -> Option<AIConversationId> {
        if let Some(conversation_id) = BlocklistAIHistoryModel::as_ref(ctx)
            .active_conversation(self.view_id)
            .and_then(|conversation| {
                conversation
                    .is_child_agent_conversation()
                    .then_some(conversation.id())
            })
        {
            return Some(conversation_id);
        }

        let mut child_conversation_ids = BlocklistAIHistoryModel::as_ref(ctx)
            .all_live_conversations_for_terminal_view(self.view_id)
            .filter(|conversation| conversation.is_child_agent_conversation())
            .map(|conversation| conversation.id());
        let child_conversation_id = child_conversation_ids.next()?;
        child_conversation_ids
            .next()
            .is_none()
            .then_some(child_conversation_id)
    }

    /// If the startup auto-open setting is enabled, auto-opens rich input for a
    /// CLI agent session. Called after creating a command-detected session or
    /// registering a listener so rich input is shown immediately.
    fn maybe_auto_open_cli_agent_rich_input(&mut self, ctx: &mut ViewContext<Self>) {
        let ai_settings = AISettings::as_ref(ctx);
        if !*ai_settings.auto_open_rich_input_on_cli_agent_start
            || !ai_settings.is_any_ai_enabled(ctx)
            || !*ai_settings.should_render_cli_agent_footer
            || !is_rich_input_chip_in_cli_toolbar(ctx)
        {
            return;
        }
        let should_open = CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .is_some_and(|s| s.should_auto_toggle_input);
        if should_open && !self.has_active_cli_agent_input_session(ctx) {
            self.open_cli_agent_rich_input(CLIAgentInputEntrypoint::AutoShow, ctx);
        }
    }

    /// Handles CLI agent session status changes from the singleton model.
    /// Sends a desktop notification when a CLI agent reaches a completed state
    /// (blocked or succeeded) and the user is in a different window.
    /// Also handles auto-show/hide of CLI agent rich input based on the
    /// `auto_toggle_rich_input` setting: closes rich input when blocked
    /// (agent requires keyboard interaction) and opens it when the agent resumes.
    pub(super) fn handle_cli_agent_sessions_event(
        &mut self,
        event: &CLIAgentSessionsModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CLIAgentSessionsModelEvent::Started {
                terminal_view_id, ..
            } if *terminal_view_id == self.view_id => {
                let mut model = self.model.lock();
                let active_block = model.block_list_mut().active_block_mut();
                active_block.enable_full_grid_clear_behavior();
                if FeatureFlag::TrimTrailingBlankLines.is_enabled() {
                    active_block.set_trim_trailing_blank_rows(true);
                }
            }
            CLIAgentSessionsModelEvent::Ended {
                terminal_view_id, ..
            } if *terminal_view_id == self.view_id => {
                let mut model = self.model.lock();
                let active_block = model.block_list_mut().active_block_mut();
                if FeatureFlag::TrimTrailingBlankLines.is_enabled() {
                    active_block.set_trim_trailing_blank_rows(false);
                }
            }
            _ => {}
        }
        if event.terminal_view_id() == self.view_id
            && matches!(
                event,
                CLIAgentSessionsModelEvent::Started { .. }
                    | CLIAgentSessionsModelEvent::StatusChanged { .. }
                    | CLIAgentSessionsModelEvent::SessionUpdated { .. }
                    | CLIAgentSessionsModelEvent::Ended { .. }
            )
        {
            self.update_pane_configuration(ctx);
            ctx.notify();
        }
        if event.terminal_view_id() == self.view_id
            && matches!(
                event,
                CLIAgentSessionsModelEvent::Started { .. }
                    | CLIAgentSessionsModelEvent::Ended { .. }
            )
        {
            self.update_git_status_subscription(ctx);
        }

        let CLIAgentSessionsModelEvent::StatusChanged {
            terminal_view_id,
            agent,
            status,
            session_context,
        } = event
        else {
            return;
        };

        if *terminal_view_id != self.view_id {
            return;
        }

        if let Some(conversation_id) = self.child_conversation_id_for_cli_status_updates(ctx) {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                history_model.update_conversation_status(
                    self.view_id,
                    conversation_id,
                    status.to_conversation_status(),
                    ctx,
                );
            });
        }

        // Auto-show/hide rich input based on the setting.
        // Only applies when the session has a plugin listener (rich status info).
        let ai_settings = AISettings::as_ref(ctx);
        if *ai_settings.auto_toggle_rich_input
            && ai_settings.is_any_ai_enabled(ctx)
            && *ai_settings.should_render_cli_agent_footer
            && is_rich_input_chip_in_cli_toolbar(ctx)
        {
            let should_auto_toggle_input = CLIAgentSessionsModel::as_ref(ctx)
                .session(self.view_id)
                .is_some_and(|s| {
                    s.listener.is_some()
                        && s.should_auto_toggle_input
                        && agent_supports_rich_status(&s.agent)
                });
            if should_auto_toggle_input {
                match status {
                    CLIAgentSessionStatus::Blocked { .. } => {
                        // Auto-close rich input when the agent is blocked
                        // (it requires direct keyboard interaction in the terminal).
                        self.close_cli_agent_rich_input(
                            CLIAgentRichInputCloseReason::AutoToggle,
                            ctx,
                        );
                    }
                    CLIAgentSessionStatus::InProgress | CLIAgentSessionStatus::Success => {
                        // Auto-open rich input when the agent resumes or completes.
                        if !self.has_active_cli_agent_input_session(ctx) {
                            self.open_cli_agent_rich_input(CLIAgentInputEntrypoint::AutoShow, ctx);
                        }
                    }
                }
            }
        }

        // Desktop notifications — only when navigated away and not in-progress.
        if !self.is_navigated_away_from_window(ctx)
            || matches!(status, CLIAgentSessionStatus::InProgress)
        {
            return;
        }

        let title = session_context
            .query
            .as_deref()
            .filter(|q| !q.is_empty())
            .or(session_context.summary.as_deref().filter(|s| !s.is_empty()))
            .unwrap_or(agent.command_prefix())
            .to_owned();
        let description = if let CLIAgentSessionStatus::Blocked { message } = status {
            message.clone().unwrap_or_default()
        } else {
            session_context.response.clone().unwrap_or_default()
        };

        let trigger = if matches!(status, CLIAgentSessionStatus::Blocked { .. }) {
            NotificationsTrigger::NeedsAttention
        } else {
            NotificationsTrigger::AgentTaskCompleted(true)
        };
        self.send_agent_desktop_notification_or_show_banner(
            trigger,
            title,
            description,
            Some(NotificationAgentVariant::CLIAgent((*agent).into())),
            ctx,
        );
    }
}
