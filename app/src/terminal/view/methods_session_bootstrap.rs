use super::*;

impl TerminalView {
    pub(super) fn handle_session_initialized(&self, ctx: &mut ViewContext<Self>) {
        // Make sure we re-render the input so we're displaying an appropriate
        // prompt.
        self.input.update(ctx, |_, ctx| {
            ctx.notify();
        });
    }

    /// Handles a session in this terminal pane completing the bootstrapping
    /// process.
    pub(super) fn handle_session_bootstrapped(
        &mut self,
        bootstrap_event: SessionBootstrappedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let session_id = bootstrap_event.session_id;
        let Some(session) = self.sessions.as_ref(ctx).get(session_id) else {
            log::error!(
                "Could not find session {session_id:?} in sessions model after \
                         being notified that the session had bootstrapped!"
            );
            return;
        };

        // Ensure that the new session's working directory and environment are persisted.
        ctx.dispatch_global_action("workspace:save_app", ());

        self.update_incompatible_configuration_banner(session.shell().plugins(), ctx);

        if let Some(subshell_info) = session.subshell_info() {
            self.warpify_state
                .add_subshell_separator(subshell_info, self.model.clone(), ctx);
        }

        self.is_login_shell_bootstrapped = true;
        self.hide_slow_bootstrap_banner(ctx);

        if self.auth_state.is_anonymous_or_logged_out()
            && !FeatureFlag::OpenWarpNewSettingsModes.is_enabled()
        {
            self.insert_anonymous_user_ai_sign_up_banner(ctx);
        }

        if self.should_display_vim_banner(&session, ctx) {
            self.insert_vim_mode_banner(ctx);
        }

        // If we were waiting to share this session once it was bootstrapped,
        // we can now attempt to share it.
        let pending_share = match self.model.lock().shared_session_status() {
            SharedSessionStatus::SharePendingPreBootstrap { source } => Some(source.clone()),
            _ => None,
        };
        if let Some(source) = pending_share {
            log::info!("Terminal bootstrapped with pending shared session; attempting to share");
            self.attempt_to_share_session(
                SharedSessionScrollbackType::All,
                None,
                source,
                false,
                ctx,
            );
        }

        if let Some(env_var_collection) = self.pending_env_var_collection.take() {
            self.invoke_environment_variables(env_var_collection, false, ctx);
        }

        // If this is a new local session, update the PATH used for MCP command execution.
        if let Some(path) = Self::local_session_path(&session) {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                // TODO: This logic is likely incorrect, as it's dynamically determining the path based on the most
                // recent session, which is not directly relevant to starting the MCP server. This caused an issue
                // on Windows where the PATH was sometimes Unix-like and other times PowerShell-like, when it should
                // always be PowerShell-like. Also an odd data flow problem to be updating an AI User Setting
                // based on a local session bootstrapping.
                if let Err(e) = settings.mcp_execution_path.set_value(Some(path), ctx) {
                    log::warn!("Failed to set MCP execution path: {e:?}");
                }
            })
        }

        let is_subshell_or_ssh = session.is_subshell_or_ssh();

        // Make sure we decorate any text that is already in the input.  We
        // need to make sure external commands have finished loading before
        // doing the decoration to ensure we don't erroneously apply error
        // underlines to valid commands.
        let input = self.input().clone();
        ctx.spawn(
            async move { session.load_external_commands().await },
            move |me, _, ctx| {
                input.update(ctx, |input, ctx| {
                    input.run_input_background_jobs(
                        InputBackgroundJobOptions::default().with_command_decoration(),
                        ctx,
                    );
                });
                me.refresh_warp_prompt(ctx);
            },
        );

        // If we were waiting for a successful warpification, it's come. Stop the timeout.
        self.warpify_state.abort_ssh_warpify_timeout();

        if bootstrap_event.subshell_info.is_some() {
            self.add_bootstrap_success_block(bootstrap_event, ctx);
        }
        self.any_session_contains_restored_remote_blocks = self.contains_restored_remote_blocks();
        self.any_session_contains_remote_blocks |= self.active_block_is_considered_remote(ctx);
        self.update_focused_terminal_info(ctx);

        if let Some(working_directory) = self.pwd_if_local(ctx) {
            CodebaseIndexManager::handle(ctx).update(ctx, |manager, _ctx| {
                let path_buf = PathBuf::from(&working_directory);
                manager.handle_session_bootstrapped(&path_buf);
            });
        }

        // At the end of bootstrapping, set the title to the title of
        // the selected conversation. If there is no selected conversation,
        // the title will default to the regular terminal title.
        self.update_pane_configuration(ctx);

        self.ignore_next_set_title_event = true;

        let auth_state = AuthStateProvider::as_ref(ctx).get();
        let is_onboarded = auth_state.is_onboarded().unwrap_or(true);
        let is_anonymous_or_logged_out = auth_state.is_anonymous_or_logged_out();
        let should_show_onboarding = FeatureFlag::AgentOnboarding.is_enabled()
            && !is_onboarded
            && !is_anonymous_or_logged_out;
        let is_launch_modal_open = OneTimeModalModel::as_ref(ctx).is_oz_launch_modal_open();

        let has_plugin_instructions_block = self.rich_content_views.iter().any(|rc| {
            matches!(
                rc.metadata(),
                Some(RichContentMetadata::PluginInstructionsBlock)
            )
        });

        if FeatureFlag::AgentView.is_enabled()
            && TerminalSettings::as_ref(ctx).should_show_zero_state_block(ctx)
            && !self.model.lock().block_list().is_restored_session()
            && !should_show_onboarding
            && self.onboarding_callout_view.is_none()
            && !is_launch_modal_open
            && !is_subshell_or_ssh
            && !has_plugin_instructions_block
        {
            let agent_view_zero_state = ctx.add_typed_action_view(|ctx| {
                TerminalViewZeroStateBlock::new(
                    &self.agent_view_controller,
                    &self.model_events_handle,
                    ctx,
                )
            });
            self.insert_rich_content(
                Some(RichContentType::TerminalViewZeroState),
                agent_view_zero_state,
                Some(RichContentMetadata::TerminalViewZeroState),
                RichContentInsertionPosition::Append {
                    insert_below_long_running_block: false,
                },
                ctx,
            );
        }

        // Now that the session is bootstrapped, update any restored AI blocks that were
        // created before bootstrapping with the shell launch data. This enables file link
        // detection and the "Open in Warp" button on code blocks in restored conversations.
        if let Some(shell_launch_data) = self.active_session.as_ref(ctx).shell_launch_data(ctx) {
            let ai_block_handles: Vec<_> = self
                .rich_content_views
                .iter()
                .filter_map(|rc| rc.ai_block_metadata())
                .map(|metadata| metadata.ai_block_handle.clone())
                .collect();
            for handle in ai_block_handles {
                handle.update(ctx, |block, ctx| {
                    block.set_shell_launch_data(Some(shell_launch_data.clone()), ctx);
                });
            }
        }

        self.refresh_warp_prompt(ctx);
        ctx.emit(Event::SessionBootstrapped);
    }

    // Helper function to get the PATH variable for a local session.
    pub(super) fn local_session_path(session: &Session) -> Option<String> {
        if matches!(session.session_type(), SessionType::Local) && session.subshell_info().is_none()
        {
            #[cfg(all(windows, feature = "local_tty"))]
            let path = {
                let path_result =
                    get_user_and_system_env_variable("PATH").map(|entry| entry.into_string());
                let result = match path_result {
                    Some(Ok(path_result)) => Some(path_result),
                    None => {
                        log::warn!("Failed to get PATH for session on Windows.");
                        None
                    }
                    Some(Err(e)) => {
                        log::warn!("Failed to convert PATH for session on Windows: `{e:?}`");
                        None
                    }
                };
                if result.is_none() {
                    if session.shell_family() == ShellFamily::PowerShell {
                        // This is a fallback for if the OsString cannot be converted to a String.
                        // We cannot accept a Posix PATH on Windows.
                        session.path().clone()
                    } else {
                        None
                    }
                } else {
                    result
                }
            };
            #[cfg(not(all(windows, feature = "local_tty")))]
            let path = session.path().clone();

            return path;
        }
        None
    }

    pub fn insert_drive_sharing_onboarding_block(
        &mut self,
        object_id: CloudObjectTypeAndId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.reset_onboarding_blocks(ctx);

        WarpDriveSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings.sharing_onboarding_block_shown.set_value(true, ctx));
        });

        let block_view_handle =
            ctx.add_view(|ctx| OnboardingDriveSharingBlock::new(object_id, ctx));

        self.insert_rich_content(
            None,
            block_view_handle,
            None,
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: false,
            },
            ctx,
        );

        send_telemetry_from_ctx!(TelemetryEvent::DriveSharingOnboardingBlockShown, ctx);
    }

    fn should_display_vim_banner(
        &self,
        session: &Arc<Session>,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        // Is this the active session?
        // We should only show the vim keybindings banner in one place at a time.
        if !self.is_active_session(ctx) {
            return false;
        }

        // Is the vim keybindings banner already open or dismissed?
        let vim_banner_displayed = self.inline_banners_state.vim_banner_state.is_some()
            || VimBannerSettings::handle(ctx).read(ctx, |banner_settings, _| {
                *banner_settings.vim_keybindings_banner_state == BannerState::Dismissed
            });

        // Have we already enabled vim keybindings?
        let vim_keybindings_enabled = AppEditorSettings::handle(ctx)
            .read(ctx, |editor_settings, _| editor_settings.vim_mode_enabled());

        if vim_banner_displayed || vim_keybindings_enabled {
            return false;
        }

        // Have we detected that vim keybindings may be wanted?
        let vi_mode_in_plugins = session.shell().plugins().contains("vi");
        let vi_mode_in_opts = session
            .shell()
            .options()
            .to_owned()
            .unwrap_or_default()
            .contains("vi_mode");

        vi_mode_in_plugins || vi_mode_in_opts
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    fn get_ps1_grid_info(&mut self) -> Option<(BlockGrid, SizeInfo)> {
        let model = self.model.lock();
        let ps1_grid_info = model
            .prompt_grid()
            .cloned()
            .zip(Some(*model.block_list().size()));
        ps1_grid_info
    }

    pub(super) fn add_agentic_suggestions_block(&mut self, ctx: &mut ViewContext<Self>) {
        self.reset_onboarding_blocks(ctx);
        self.block_onboarding_active = true;
        ctx.focus_self();
        let session_id_opt = self.active_block_session_id();
        let shell_type = self.active_session_shell_type(ctx);

        if let (Some(shell_type), Some(session_id)) = (shell_type, session_id_opt) {
            let terminal_view_handle = ctx.handle();
            let onboarding_agentic_suggestions_block = ctx.add_typed_action_view(|ctx| {
                OnboardingAgenticSuggestionsBlock::new(
                    session_id,
                    shell_type,
                    terminal_view_handle,
                    self.model_events_handle.clone(),
                    self.ai_action_model.clone(),
                    ctx,
                )
            });
            self.onboarding_agentic_suggestions_block =
                Some(onboarding_agentic_suggestions_block.clone());

            ctx.subscribe_to_view(
                &onboarding_agentic_suggestions_block,
                move |me, _, event, ctx| {
                    me.handle_onboarding_agentic_suggestions_block_event(event, ctx);
                },
            );

            self.insert_rich_content(
                None,
                onboarding_agentic_suggestions_block.clone(),
                Some(RichContentMetadata::OnboardingAgenticSuggestions {
                    agentic_suggestions_block_handle: onboarding_agentic_suggestions_block,
                }),
                RichContentInsertionPosition::Append {
                    insert_below_long_running_block: false,
                },
                ctx,
            );
        } else {
            ctx.subscribe_to_model(&History::handle(ctx), |me, _, event, ctx| match event {
                HistoryEvent::Initialized(_) => {
                    if me.pending_onboarding_agentic_suggestions_block {
                        me.add_agentic_suggestions_block(ctx);
                        me.pending_onboarding_agentic_suggestions_block = false;
                    }
                }
            });
        }

        #[cfg(feature = "voice_input")]
        voice_input::VoiceInput::handle(ctx).update(ctx, |voice_input, _| {
            voice_input.should_suppress_new_feature_popup = true;
        });
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub(super) fn add_settings_import_block(&mut self, ctx: &mut ViewContext<Self>) {
        self.block_onboarding_active = true;
        let current_block_view_handle = ctx.add_typed_action_view(SettingsImportView::new);
        self.settings_import_onboarding_block = Some(current_block_view_handle.clone());

        ctx.subscribe_to_view(
            &current_block_view_handle,
            move |terminal_view, settings_import_view_handle, event, ctx| match event {
                SettingsImportEvent::Completed(true) => {
                    terminal_view.add_prompt_block(ctx);
                }
                SettingsImportEvent::NoConfigsFound => {
                    // In the case where no settings were found to import, we want to remove the settings import block.
                    terminal_view
                        .model
                        .lock()
                        .block_list_mut()
                        .remove_rich_content(settings_import_view_handle.id());

                    terminal_view.add_prompt_block(ctx);
                }
                _ => {
                    terminal_view.add_prompt_block(ctx);
                }
            },
        );

        self.insert_rich_content(
            None,
            current_block_view_handle,
            None,
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: false,
            },
            ctx,
        );

        #[cfg(feature = "voice_input")]
        voice_input::VoiceInput::handle(ctx).update(ctx, |voice_input, _| {
            voice_input.should_suppress_new_feature_popup = true;
        });
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub(super) fn add_prompt_block(&mut self, ctx: &mut ViewContext<Self>) {
        let ps1_grid_info = self.get_ps1_grid_info();
        let current_block_view_handle =
            ctx.add_typed_action_view(|_| OnboardingPromptBlock::new(ps1_grid_info));
        self.onboarding_prompt_block = Some(current_block_view_handle.clone());

        self.insert_rich_content(
            None,
            current_block_view_handle,
            None,
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: false,
            },
            ctx,
        );

        if self.block_onboarding_active {
            #[cfg(feature = "voice_input")]
            {
                voice_input::VoiceInput::handle(ctx).update(ctx, |voice_input, _| {
                    voice_input.should_suppress_new_feature_popup = true;
                });
            }
        }
    }

    fn handle_onboarding_agentic_suggestions_block_event(
        &mut self,
        event: &OnboardingAgenticSuggestionsBlockEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            OnboardingAgenticSuggestionsBlockEvent::RunAgentModeCommand { prompt, chip_type } => {
                let static_query_type = if matches!(chip_type, OnboardingChipType::Other) {
                    Some(StaticQueryType::CustomOnboardingRequest)
                } else {
                    None
                };

                self.ai_controller.update(ctx, move |controller, ctx| {
                    controller.send_user_query_in_new_conversation(
                        prompt.clone(),
                        static_query_type,
                        EntrypointType::Onboarding {
                            chip_type: *chip_type,
                        },
                        None,
                        ctx,
                    )
                });

                send_telemetry_from_ctx!(
                    TelemetryEvent::AgenticOnboardingBlockSelected {
                        block_type: *chip_type,
                    },
                    ctx
                );
            }
        }
    }

    pub fn interrupt_onboarding_blocks(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(onboarding_prompt_block_handle) = &self.onboarding_prompt_block {
            onboarding_prompt_block_handle.update(ctx, |onboarding_prompt_block, block_ctx| {
                onboarding_prompt_block.interrupt_block(block_ctx);
            })
        }

        if let Some(settings_import_onboarding_block_handle) =
            &self.settings_import_onboarding_block
        {
            settings_import_onboarding_block_handle.update(ctx, |settings_import_view, ctx| {
                settings_import_view.interrupt_block(ctx);
            })
        }

        if let Some(agentic_suggestions_block_handle) = &self.onboarding_agentic_suggestions_block {
            agentic_suggestions_block_handle.update(ctx, |agentic_suggestions_block, ctx| {
                agentic_suggestions_block.interrupt_block(ctx);
            })
        }

        self.reset_onboarding_blocks(ctx);
    }

    /// Opens a folder that the user may or may not have opened in the past
    pub fn open_repo_folder(
        &mut self,
        path: String,
        should_init_repo: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        let path_buf = PathBuf::from(&path);

        if should_init_repo {
            self.maybe_set_pending_repo_init_path(path_buf);
        }

        self.input.update(ctx, |input, ctx| {
            input.try_execute_command(format!("cd \"{path}\"").as_str(), ctx);
        });

        self.toggle_left_panel_file_tree(true, ctx);
    }

    pub fn create_new_project(&mut self, prompt: String, ctx: &mut ViewContext<Self>) {
        self.input.update(ctx, |input, ctx| {
            input.initiate_create_new_project(prompt, ctx);
        });
    }

    pub fn agent_clone_repository(&mut self, url: String, ctx: &mut ViewContext<Self>) {
        self.input.update(ctx, |input, ctx| {
            input.initiate_clone_repository(url, ctx);
        });
    }

    pub fn maybe_set_pending_repo_init_path(&mut self, path: PathBuf) {
        self.on_next_block_completed(move |me, ctx| {
            if me
                .current_local_repo_path()
                .is_some_and(|repo_path| repo_path == path)
            {
                me.init_project_and_suppress_banners(path, ctx);
            }
        });
    }

    // Initialize project for a path and suppress the agent mode setup banner for that path. This also auto-opens
    // the code-review pane after the initialization step completes.
    fn init_project_and_suppress_banners(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        log::info!("Indexing and running /init for new repo at {path:?}");

        // Ensure we don't hit speedumps - Mark this as "already shown and dismissed"
        // This method is used when opening a new repo that the user has selected directly.
        self.mark_agent_init_callout_as_shown_for_directory(&path, ctx);
        AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
            let mut dismissed_paths = ai_settings
                .codebase_index_speedbump_banner_dismissed_for_repo_paths
                .clone();
            if !dismissed_paths.contains(&path) {
                dismissed_paths.push(path.clone());
                let _ = ai_settings
                    .codebase_index_speedbump_banner_dismissed_for_repo_paths
                    .set_value(dismissed_paths, ctx);
            }
        });

        self.init_project(true, ctx);
    }

    // Show or hide codebase index speedbump depending when a settings change happens.
    pub(super) fn check_codebase_index_speedbump_on_settings_changed(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(working_directory) = self.pwd_if_local(ctx) {
            let path_buf = PathBuf::from(&working_directory);
            self.update_repo_banner_state(path_buf, ctx);
        }
    }

    pub(super) fn summarize_conversation(&mut self, ctx: &mut ViewContext<Self>) {
        self.ai_controller.update(ctx, |controller, ctx| {
            controller
                .send_slash_command_request(SlashCommandRequest::Summarize { prompt: None }, ctx);
        });
    }

    pub(super) fn init_project(
        &mut self,
        open_code_review_pane_after_rule_generation: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.has_active_init_project(ctx) {
            return;
        }

        let Some(pwd_path) = self
            .pwd()
            .and_then(|pwd| Path::new(&pwd).canonicalize().ok())
        else {
            return;
        };

        let path_env_var = self
            .active_block_session_id()
            .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id))
            .and_then(|session| session.path().clone());

        // Create new conversation for init flow (this ensures we enter the agent view)
        let Some(conversation_id) = (if FeatureFlag::AgentView.is_enabled() {
            self.enter_agent_view_for_new_conversation(None, AgentViewEntryOrigin::SlashInit, ctx);
            self.agent_view_controller()
                .as_ref(ctx)
                .agent_view_state()
                .active_conversation_id()
        } else {
            Some(
                BlocklistAIHistoryModel::handle(ctx).update(ctx, |history_model, ctx| {
                    history_model.start_new_conversation(self.view_id, false, false, false, ctx)
                }),
            )
        }) else {
            return;
        };

        // Set fallback title since /init may have no initial query
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _ctx| {
            if let Some(conversation) = history.conversation_mut(&conversation_id) {
                conversation.set_fallback_display_title("Project setup".to_string());
            }
        });

        let init_model = ctx.add_model(|ctx| InitProjectModel::new(pwd_path, path_env_var, ctx));
        self.active_init_project_model = Some(init_model.clone());

        ctx.subscribe_to_model(&init_model, move |me, model, event, ctx| {
            match event {
                InitProjectModelEvent::InsertStep(kind) => {
                    me.insert_init_step_block(*kind, model.clone(), ctx);
                    me.redetermine_terminal_focus(ctx);
                }
                InitProjectModelEvent::StepCompleted(_) => {}
                InitProjectModelEvent::Cancelled => {
                    me.active_init_project_model = None;
                    // Mark conversation as cancelled
                    //
                    // We have to do this to handle the case where an init flow is just made up
                    // of `InitProjectBlock`s (no actual conversation steps were triggered) -
                    // the controller doesn't update the conversation status in those cases, so
                    // without this we'd see an "in progress" conversation.
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                        history.update_conversation_status(
                            me.view_id,
                            conversation_id,
                            ConversationStatus::Cancelled,
                            ctx,
                        );
                    });
                    me.redetermine_terminal_focus(ctx);
                }
                InitProjectModelEvent::InitCompleted => {
                    me.active_init_project_model = None;
                    // Mark conversation as success
                    //
                    // We have to do this to handle the case where an init flow is just made up
                    // of `InitProjectBlock`s (no actual conversation steps were triggered) -
                    // the controller doesn't update the conversation status in those cases, so
                    // without this we'd see an "in progress" conversation.
                    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                        history.update_conversation_status(
                            me.view_id,
                            conversation_id,
                            ConversationStatus::Success,
                            ctx,
                        );
                    });
                    #[cfg(feature = "local_fs")]
                    me.start_lsp_server_in_active_pwd(ctx);
                    me.redetermine_terminal_focus(ctx);
                    ctx.emit(Event::OnboardingInitCompleted);
                }
                InitProjectModelEvent::GenerateProjectRules => {
                    me.ai_controller.update(ctx, |controller, ctx| {
                        controller.send_ai_input_with_context(
                            |context| AIAgentInput::InitProjectRules {
                                context,
                                display_query: None,
                            },
                            ctx,
                        );
                    });

                    // Mark step completed when conversation finishes
                    let model = model.clone();
                    me.on_next_conversation_finished(move |me, _reason, ctx| {
                        model.update(ctx, |m, ctx| {
                            m.mark_step_completed(
                                InitStepKind::ProjectScopedRules,
                                InitActionResult::ProjectScopedRules(
                                    ProjectScopedRulesResult::GenerateNew {
                                        mouse_state: Default::default(),
                                        button_disabled: false,
                                    },
                                ),
                                ctx,
                            );
                        });

                        if open_code_review_pane_after_rule_generation {
                            me.toggle_code_review_pane(
                                GitDeltaPreference::Always,
                                CodeReviewPaneEntrypoint::AgentModeCompleted,
                                None,
                                false, /* focus_new_pane */
                                ctx,
                            );
                        }
                    });
                }
                InitProjectModelEvent::RegenerateProjectRules => {
                    me.ai_controller.update(ctx, |controller, ctx| {
                        controller.send_ai_input_with_context(
                            |context| AIAgentInput::InitProjectRules {
                                context,
                                display_query: None,
                            },
                            ctx,
                        );
                    });
                    // Clicking this button doesn't mark the step as running, so we don't need to
                    // register anything to mark the step as complete.
                }
                InitProjectModelEvent::ViewCodebaseContextStatus => {
                    ctx.emit(Event::OpenSettings(SettingsSection::CodeIndexing));
                }
                InitProjectModelEvent::LanguageServerInstalledAndEnabled => {
                    #[cfg(feature = "local_fs")]
                    me.start_lsp_server_in_active_pwd(ctx);
                }
                InitProjectModelEvent::CreateEnvironment => {
                    me.ai_controller.update(ctx, |controller, ctx| {
                        controller.send_ai_input_with_context(
                            |context| AIAgentInput::CreateEnvironment {
                                context,
                                display_query: None,
                                repo_paths: vec![".".to_string()],
                            },
                            ctx,
                        );
                    });
                }
                InitProjectModelEvent::EnvironmentCreated => {
                    let model = model.clone();
                    me.on_next_conversation_finished(move |_me, _reason, ctx| {
                        model.update(ctx, |m, ctx| {
                            m.mark_step_completed(
                                InitStepKind::CreateEnvironment,
                                init_project::InitActionResult::CreateEnvironment(
                                    init_project::CreateEnvironmentResult::Created,
                                ),
                                ctx,
                            );
                        });
                    });
                }
            }
        });
        // After subscribing, start the /init flow
        init_model.update(ctx, |model, ctx| {
            model.start(ctx);
        });
    }

    /// Insert an InitStepBlock for the given step kind
    pub(super) fn insert_init_step_block(
        &mut self,
        kind: InitStepKind,
        model: ModelHandle<InitProjectModel>,
        ctx: &mut ViewContext<Self>,
    ) {
        let step_block = ctx.add_typed_action_view(move |ctx| InitStepBlock::new(kind, model, ctx));

        self.insert_rich_content(
            None,
            step_block.clone(),
            Some(RichContentMetadata::InitStep {
                step_kind: kind,
                block_handle: step_block,
            }),
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: true,
            },
            ctx,
        );
    }

    /// Try to focus the most recent init step block that's awaiting user input
    pub(super) fn try_focus_active_init_step(&mut self, ctx: &mut ViewContext<Self>) {
        for rc in self.rich_content_views.iter().rev() {
            if let Some(block_handle) = rc.init_step_block_handle() {
                block_handle.update(ctx, |block, ctx| block.try_steal_focus(ctx));
                return;
            }
        }
    }

    /// Open the Environment Management pane.
    pub(super) fn open_environment_management_pane(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(Event::OpenEnvironmentManagementPane);
    }

    /// Check if completed command was `warp environment create` and emit event if successful
    pub(super) fn maybe_handle_environment_create_command(
        &mut self,
        block_completed: &UserBlockCompleted,
        ctx: &mut ViewContext<Self>,
    ) {
        let cli_name = ChannelState::channel().cli_command_name();
        let cmd = &block_completed.command;
        let is_env_create =
            cmd.contains(cli_name) && cmd.contains("environment") && cmd.contains("create");

        if !is_env_create || !block_completed.serialized_block.exit_code.was_successful() {
            return;
        }

        if let Some(model) = &self.active_init_project_model {
            model.update(ctx, |_, ctx| {
                ctx.emit(InitProjectModelEvent::EnvironmentCreated);
            });
        }
    }

    pub(super) fn enter_environment_setup_selector(
        &mut self,
        args: Vec<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        // If arguments are provided (repo paths/URLs), skip the mode selector and go directly
        // to the local agent flow
        if !args.is_empty() {
            self.setup_cloud_environment_and_start(args, ctx);
            return;
        }

        // If already in ambient agent mode, skip the mode selector and go
        // directly to the environment management pane
        if FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(ctx).is_active()
            && self.is_ambient_agent_session(ctx)
        {
            self.open_environment_management_pane(ctx);
            return;
        }

        // No arguments provided and not in agent view - show the mode selector modal
        // Note: We don't call close_overlays here because this action may be dispatched
        // from within the input view (e.g., slash command execution), and calling
        // close_overlays would attempt to update the input view while it's already
        // being updated, causing a circular view update panic.
        self.is_environment_setup_mode_selector_open = true;
        ctx.emit(Event::EnvironmentSetupModeSelectorToggled { is_open: true });
        ctx.notify();
        // Focus the mode selector so it can receive keyboard events (ESC to dismiss)
        ctx.focus(&self.environment_setup_mode_selector);
    }

    pub(super) fn setup_cloud_environment(
        &mut self,
        args: Vec<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if FeatureFlag::AgentView.is_enabled()
            && !self.agent_view_controller.as_ref(ctx).is_active()
        {
            self.enter_agent_view_for_new_conversation(
                None,
                AgentViewEntryOrigin::CreateEnvironment,
                ctx,
            );
        }

        let repos = args;
        let (button_label, use_current_dir) = if !repos.is_empty() {
            (
                format!(
                    "Create environment using the supplied repos: {}",
                    repos.join(", ")
                ),
                false,
            )
        } else {
            #[cfg(feature = "local_fs")]
            let is_repo = {
                if let Some(pwd_path) = self
                    .pwd()
                    .and_then(|pwd| Path::new(&pwd).canonicalize().ok())
                {
                    DetectedRepositories::as_ref(ctx)
                        .get_root_for_path(&LocalOrRemotePath::Local(pwd_path))
                        .is_some()
                } else {
                    false
                }
            };

            #[cfg(not(feature = "local_fs"))]
            let is_repo = false;

            if is_repo {
                (
                    "Create environment using the current working dir as repo".to_string(),
                    true,
                )
            } else {
                ("Create environment without any repos".to_string(), false)
            }
        };

        let init_env_block = ctx.add_typed_action_view(move |ctx| {
            InitEnvironmentBlock::new(button_label, repos, use_current_dir, ctx)
        });
        ctx.subscribe_to_view(&init_env_block, move |me, block, event, ctx| match event {
            InitEnvironmentBlockEvent::StartSetup(repos, use_current_dir) => {
                log::info!("TerminalView: received StartSetup event from InitEnvironmentBlock");

                // Remove the block from the UI now that setup is starting
                me.model
                    .lock()
                    .block_list_mut()
                    .remove_rich_content(block.id());

                me.start_cloud_environment_setup(repos.to_vec(), *use_current_dir, ctx);
            }
        });

        self.insert_rich_content(
            None,
            init_env_block.clone(),
            Some(RichContentMetadata::InitEnvironment {
                block_handle: init_env_block,
            }),
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: true,
            },
            ctx,
        );

        self.redetermine_global_focus(ctx);
    }

    pub(super) fn handle_environment_setup_mode_selector_event(
        &mut self,
        event: &EnvironmentSetupModeSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EnvironmentSetupModeSelectorEvent::Selected(mode) => {
                self.is_environment_setup_mode_selector_open = false;
                ctx.emit(Event::EnvironmentSetupModeSelectorToggled { is_open: false });

                match mode {
                    EnvironmentSetupMode::RemoteGitHub => {
                        // Open the environment management pane (form-based flow)
                        self.open_environment_management_pane(ctx);
                    }
                    EnvironmentSetupMode::LocalRepositories => {
                        // Use the agent-based flow, directly starting without confirmation
                        // When the mode selector is shown, no args were provided
                        self.setup_cloud_environment_and_start(Vec::new(), ctx);
                    }
                }
                self.redetermine_global_focus(ctx);
                ctx.notify();
            }
            EnvironmentSetupModeSelectorEvent::Dismissed => {
                self.is_environment_setup_mode_selector_open = false;
                ctx.emit(Event::EnvironmentSetupModeSelectorToggled { is_open: false });
                self.redetermine_global_focus(ctx);
                ctx.notify();
            }
        }
    }

    pub(super) fn setup_cloud_environment_and_start(
        &mut self,
        args: Vec<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if FeatureFlag::AgentView.is_enabled()
            && !self.agent_view_controller.as_ref(ctx).is_active()
        {
            self.enter_agent_view_for_new_conversation(
                None,
                AgentViewEntryOrigin::CreateEnvironment,
                ctx,
            );
        }

        let repos = args;

        #[cfg(feature = "local_fs")]
        let use_current_dir = repos.is_empty()
            && self
                .pwd()
                .and_then(|pwd| Path::new(&pwd).canonicalize().ok())
                .is_some_and(|pwd_path| {
                    DetectedRepositories::as_ref(ctx)
                        .get_root_for_path(&LocalOrRemotePath::Local(pwd_path))
                        .is_some()
                });

        #[cfg(not(feature = "local_fs"))]
        let use_current_dir = false;

        self.start_cloud_environment_setup(repos, use_current_dir, ctx);
    }

    pub(super) fn start_cloud_environment_setup(
        &mut self,
        repos: Vec<String>,
        use_current_dir: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        // Clear input and switch to Agent Mode
        self.input.update(ctx, |input, ctx| {
            input
                .editor()
                .update(ctx, |editor, ctx| editor.clear_buffer(ctx));
            input.set_input_mode_agent(false, ctx);
        });

        // Send the CreateEnvironment request (shows "/create-environment" instead of full prompt)
        self.ai_controller.update(ctx, |controller, ctx| {
            controller.send_slash_command_request(
                SlashCommandRequest::CreateEnvironment {
                    repos,
                    use_current_dir,
                },
                ctx,
            );
        });

        ctx.notify();
    }

    pub(super) fn mark_agent_init_callout_as_shown_for_directory(
        &self,
        directory: &Path,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut shown_repo_paths = AISettings::as_ref(ctx)
            .agent_mode_setup_banner_shown_for_repo_paths
            .clone();
        if shown_repo_paths
            .iter()
            .any(|shown_path| shown_path == directory)
        {
            return;
        }
        shown_repo_paths.push(directory.to_path_buf());
        AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
            if let Err(e) = ai_settings
                .agent_mode_setup_banner_shown_for_repo_paths
                .set_value(shown_repo_paths, ctx)
            {
                log::error!("Failed to persist 'Agent Mode setup banner shown' setting: {e}");
            }
        });
    }

    pub(super) fn reset_onboarding_blocks(&mut self, ctx: &mut ViewContext<Self>) {
        self.block_onboarding_active = false;
        self.onboarding_prompt_block = None;
        self.settings_import_onboarding_block = None;
        self.onboarding_agentic_suggestions_block = None;

        #[cfg(feature = "voice_input")]
        voice_input::VoiceInput::handle(ctx).update(ctx, |voice_input, _| {
            voice_input.should_suppress_new_feature_popup = false;
        });
        let _ = ctx;
    }

    /// Returns the save position ID for the agent view zero state, if one exists.
    pub(super) fn agent_view_zero_state_save_position_id(
        &self,
        app: &AppContext,
    ) -> Option<String> {
        self.agent_view_controller
            .as_ref(app)
            .agent_view_state()
            .zero_state_position_id()
    }

    /// Gets the selected text from the terminal, if any.
    pub fn selected_text(&self, ctx: &AppContext) -> Option<String> {
        let semantic_selection = SemanticSelection::handle(ctx).as_ref(ctx);
        let input_mode = *InputModeSettings::handle(ctx)
            .as_ref(ctx)
            .input_mode
            .value();
        let inverted = input_mode.is_inverted_blocklist();
        self.model
            .lock()
            .selection_to_string(semantic_selection, inverted, ctx)
    }

    /// Gets the selected text from the terminal input editor, if any.
    pub fn selected_text_from_input(&self, ctx: &AppContext) -> Option<String> {
        let text = self
            .input
            .as_ref(ctx)
            .editor()
            .as_ref(ctx)
            .selected_text(ctx);
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }
}
