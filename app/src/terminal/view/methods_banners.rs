use super::*;

impl TerminalView {
    pub(super) fn insert_notifications_discovery_banner(
        &mut self,
        trigger: NotificationsTrigger,
        ctx: &mut ViewContext<Self>,
    ) {
        // Don't show if the user has dismissed the banner in this session.
        if matches!(
            self.inline_banners_state.notifications_discovery_banner,
            NotificationsDiscoveryBanner::Closed
        ) {
            return;
        }

        let banner = &self.inline_banners_state.notifications_discovery_banner;
        // Prevent stacking multiple banners or leaving empty space.
        if let NotificationsDiscoveryBanner::Open { state, .. } = banner {
            self.model
                .lock()
                .block_list_mut()
                .remove_inline_banner(state.banner_id);
        }

        let banner_id = self.inline_banners_state.next_banner_id();
        self.inline_banners_state.notifications_discovery_banner =
            NotificationsDiscoveryBanner::Open {
                trigger,
                state: NotificationsDiscoveryBannerState {
                    banner_id,
                    mouse_states: Default::default(),
                },
                request_outcome: None,
            };
        self.model
            .lock()
            .block_list_mut()
            .append_inline_banner(InlineBannerItem::new(
                banner_id,
                InlineBannerType::NotificationsDiscovery,
            ));

        let a11y_content = AccessibilityContent::new(
            trigger.discovery_banner_copy(),
            "You can enable notifications through the command palette.",
            WarpA11yRole::TextRole,
        );
        ctx.emit_a11y_content(a11y_content);

        send_telemetry_from_ctx!(TelemetryEvent::ShowNotificationsDiscoveryBanner, ctx);
        ctx.notify();
    }

    /// Inserts a notifications error banner into the block list.
    pub(super) fn insert_notifications_error_banner(&mut self, ctx: &mut ViewContext<Self>) {
        let banner_id = self.inline_banners_state.next_banner_id();

        self.inline_banners_state
            .notifications_error_banner
            .banner_type = NotificationsErrorBannerType::Open {
            state: NotificationsErrorBannerState {
                banner_id,
                mouse_states: Default::default(),
            },
        };
        self.model
            .lock()
            .block_list_mut()
            .append_inline_banner(InlineBannerItem::new(
                banner_id,
                InlineBannerType::NotificationsError,
            ));

        let banner_title = self
            .inline_banners_state
            .notifications_error_banner
            .error
            .as_ref()
            .map(|e| e.notifications_error_banner_title())
            .unwrap_or("Error sending notification");

        let a11y_content = AccessibilityContent::new(
            banner_title,
            "Make sure you have enabled access for Warp notifications in System Preferences.",
            WarpA11yRole::TextRole,
        );
        ctx.emit_a11y_content(a11y_content);

        send_telemetry_from_ctx!(TelemetryEvent::ShowNotificationsErrorBanner, ctx);

        ctx.notify();
    }

    pub(super) fn insert_command_correction(
        &mut self,
        correction: &Correction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.input.update(ctx, |input, ctx| {
            input.replace_buffer_content(correction.command.as_str(), ctx);
            ctx.notify()
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::CommandCorrection {
                event: CommandCorrectionEvent::Accepted {
                    via: CommandCorrectionAcceptedType::Banner,
                    rule: correction.rule_applied.to_str(),
                }
            },
            ctx
        );
    }

    /// Returns the view type for prompt suggestion telemetry based on whether agent view is active.
    pub(super) fn prompt_suggestion_view_type(
        &self,
        ctx: &ViewContext<Self>,
    ) -> PromptSuggestionViewType {
        if FeatureFlag::AgentView.is_enabled() && self.agent_view_controller.as_ref(ctx).is_active()
        {
            PromptSuggestionViewType::AgentView
        } else {
            PromptSuggestionViewType::TerminalView
        }
    }

    pub(super) fn resolve_prompt_suggestion(
        &mut self,
        resolution: PromptSuggestionResolution,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let interaction_source = match resolution {
            PromptSuggestionResolution::Accept { interaction_source } => interaction_source,
            PromptSuggestionResolution::Reject { ctrl_c } => {
                // ctrl-c shouldn't clear prompt suggestions, but all other rejections should.
                if !ctrl_c {
                    self.clear_prompt_suggestions(ctx);
                }
                return false;
            }
        };

        // Return early if we've run out of AI usage.
        if !AIRequestUsageModel::as_ref(ctx).has_any_ai_remaining(ctx) {
            return false;
        }

        let Some(banner_state) = &self.inline_banners_state.prompt_suggestions_banner else {
            return false;
        };

        // Return early if the banner is not visible to the user.
        if banner_state.should_hide {
            return false;
        }

        let view = self.prompt_suggestion_view_type(ctx);
        let suggestion = &banner_state.prompt_suggestion;
        let prompt = suggestion.prompt.clone();
        let suggestion_id = suggestion.id.clone();
        let is_static_suggestion = suggestion.static_prompt_suggestion_name.is_some();
        let trigger = banner_state.trigger.clone();
        let should_start_new_conversation = suggestion.should_start_new_conversation;
        let conversation_id = banner_state.conversation_id;
        let trigger_block_id = trigger.as_ref().and_then(|t| t.block_id());
        log::debug!(
            "[passive-suggestions] accepting prompt suggestion: trigger={}, trigger_block_id={}",
            if trigger.is_some() { "Some" } else { "None" },
            if trigger_block_id.is_some() {
                "Some"
            } else {
                "None"
            },
        );

        if FeatureFlag::PromptSuggestionsViaMAA.is_enabled() {
            let conversation_id = if let Some(conversation_id) = conversation_id {
                conversation_id
            } else {
                match self.try_enter_agent_view(
                    None,
                    AgentViewEntryOrigin::AcceptedPromptSuggestion,
                    None,
                    ctx,
                ) {
                    Ok(conversation_id) => {
                        if let Some(block_id) = trigger_block_id.as_ref() {
                            self.associate_and_promote_block_for_conversation(
                                block_id.clone(),
                                conversation_id,
                                ctx,
                            );
                        }
                        conversation_id
                    }
                    Err(e) => {
                        log::error!("Failed to enter agent view for passive code diff: {e:?}");
                        return false;
                    }
                }
            };

            self.ai_controller.update(ctx, |controller, ctx| {
                controller.send_passive_suggestion_result(
                    Some(conversation_id),
                    PassiveSuggestionResultType::Prompt { prompt },
                    trigger,
                    ctx,
                );
            });
        } else {
            if let Some(PassiveSuggestionTrigger::ShellCommandCompleted(c)) = &banner_state.trigger
            {
                let block_id = c.executed_shell_command.id.clone();
                self.ai_context_model.update(ctx, |context_model, ctx| {
                    context_model.set_pending_context_block_ids(vec![block_id], true, ctx);
                });
            }
            // When `should_start_new_conversation` is false and agent view is already
            // active, continue in the existing conversation rather than starting a new one.
            let conversation_id = if !should_start_new_conversation {
                self.agent_view_controller
                    .as_ref(ctx)
                    .agent_view_state()
                    .active_conversation_id()
            } else {
                None
            };
            self.enter_agent_view(
                Some(prompt),
                conversation_id,
                AgentViewEntryOrigin::AcceptedPromptSuggestion,
                ctx,
            );
        }

        // Send telemetry.
        if is_static_suggestion {
            send_telemetry_from_ctx!(
                TelemetryEvent::StaticPromptSuggestionAccepted {
                    id: suggestion_id,
                    view,
                    interaction_source,
                },
                ctx
            );
        } else {
            send_telemetry_from_ctx!(
                TelemetryEvent::PromptSuggestionAccepted {
                    id: suggestion_id,
                    view,
                    interaction_source,
                },
                ctx
            );
        }

        true
    }

    /// Try clearing agent mode query banner's passive code generation state.
    /// Called when a suggested code diff fails and we need to fall back to prompt suggestions.
    pub(super) fn try_clear_prompt_suggestions_banner_code_state(
        &mut self,
        fallback_reason: PromptSuggestionFallbackReason,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(banner) = &mut self.inline_banners_state.prompt_suggestions_banner {
            banner.should_hide = false;
            banner.prompt_suggestion.coding_query_context = None;
            self.input.update(ctx, |input, ctx| {
                input.maybe_set_prompt_suggestions_banner_state_should_hide(false);
                input.notify_and_notify_children(ctx);
            });
            send_telemetry_from_ctx!(
                TelemetryEvent::SuggestedCodeDiffFailed {
                    prompt_suggestion_id: banner.prompt_suggestion.id.clone(),
                    reason: fallback_reason,
                },
                ctx
            );
        }
    }

    pub(super) fn associate_and_promote_block_for_conversation(
        &mut self,
        block_id: BlockId,
        conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        self.ai_context_model.update(ctx, |context_model, ctx| {
            context_model.set_pending_context_block_ids([block_id.clone()], false, ctx);
        });
        let associated_blocks = self
            .model
            .lock()
            .block_list_mut()
            .associate_blocks_with_conversation([&block_id].into_iter(), conversation_id);
        self.model
            .lock()
            .block_list_mut()
            .promote_blocks_to_attached_from_conversation(conversation_id);

        if let Some(sender) = GlobalResourceHandlesProvider::as_ref(ctx)
            .get()
            .model_event_sender
            .as_ref()
        {
            for (block_id, agent_view_visibility) in associated_blocks {
                if let Err(e) =
                    sender.send(persistence::ModelEvent::UpdateBlockAgentViewVisibility {
                        block_id: block_id.to_string(),
                        agent_view_visibility: agent_view_visibility.into(),
                    })
                {
                    log::error!("Error sending UpdateBlockAgentViewVisibility event: {e:?}");
                }
            }
        }
    }

    pub(super) fn passive_code_diffs_enabled(ctx: &mut ViewContext<Self>) -> bool {
        // Prompt suggestions must be enabled since the current implementation of passive code diffs
        // depends on generating a prompt suggestion.
        let ai_settings = AISettings::as_ref(ctx);
        let is_prompt_suggestions_enabled = ai_settings.is_prompt_suggestions_enabled(ctx);
        let is_setting_enabled = ai_settings.is_code_suggestions_enabled(ctx);
        let is_setting_toggleable = UserWorkspaces::as_ref(ctx).is_code_suggestions_toggleable();
        is_prompt_suggestions_enabled && is_setting_enabled && is_setting_toggleable
    }

    pub(super) fn insert_alias_expansion_banner(
        &mut self,
        aliased_command: AliasedCommand,
        ctx: &mut ViewContext<Self>,
    ) {
        if let AliasExpansionBanner::Open { .. } = self.inline_banners_state.alias_expansion_banner
        {
            // We only show this banner once to the user.
            log::warn!("Tried to insert more than one alias expansion banner");
            return;
        }
        let banner_id = self.inline_banners_state.next_banner_id();
        self.inline_banners_state.alias_expansion_banner = AliasExpansionBanner::Open {
            state: AliasExpansionBannerState {
                id: banner_id,
                aliased_command,
                yes_button_mouse_state: Default::default(),
                no_button_mouse_state: Default::default(),
            },
        };

        send_telemetry_from_ctx!(TelemetryEvent::ShowAliasExpansionBanner, ctx);

        self.model
            .lock()
            .block_list_mut()
            .append_inline_banner(InlineBannerItem::new(
                banner_id,
                InlineBannerType::AliasExpansion,
            ));
        ctx.notify();
    }

    /// Inserts a vim keybinding banner into the blocklist.
    pub(super) fn insert_vim_mode_banner(&mut self, ctx: &mut ViewContext<Self>) {
        let banner_id = self.inline_banners_state.next_banner_id();
        self.inline_banners_state.vim_banner_state = Some(VimModeBannerState {
            id: banner_id,
            yes_button_mouse_state: Default::default(),
            no_button_mouse_state: Default::default(),
        });

        self.model
            .lock()
            .block_list_mut()
            .append_inline_banner(InlineBannerItem::new(banner_id, InlineBannerType::VimMode));

        send_telemetry_from_ctx!(TelemetryEvent::ShowVimKeybindingsBanner, ctx);

        ctx.notify();
    }

    pub(super) fn remove_vim_mode_banner(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(banner_state) = self.inline_banners_state.vim_banner_state.take() {
            self.model
                .lock()
                .block_list_mut()
                .remove_inline_banner(banner_state.id);
        }
        ctx.notify();
    }

    pub(super) fn enable_vim_keybindings(&mut self, ctx: &mut ViewContext<Self>) {
        AppEditorSettings::handle(ctx).update(ctx, |editor_settings, ctx| {
            if editor_settings.vim_mode.set_value(true, ctx).is_ok() {
                send_telemetry_from_ctx!(TelemetryEvent::EnableVimKeybindingsFromBanner, ctx);
            }
        });
    }

    pub(super) fn handle_vim_banner_action(
        &mut self,
        action: VimModeBannerAction,
        ctx: &mut ViewContext<Self>,
    ) {
        if action == VimModeBannerAction::Enable {
            self.enable_vim_keybindings(ctx);
        } else {
            send_telemetry_from_ctx!(TelemetryEvent::DismissVimKeybindingsBanner, ctx);
        }
        self.remove_vim_mode_banner(ctx);
        VimBannerSettings::handle(ctx).update(ctx, |banner_settings, model_ctx| {
            report_if_error!(banner_settings
                .vim_keybindings_banner_state
                .set_value(BannerState::Dismissed, model_ctx));
        });
    }

    pub(super) fn agent_mode_setup_speedbump_banner_action(
        &mut self,
        action: AgentModeSetupSpeedbumpBannerAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            AgentModeSetupSpeedbumpBannerAction::Close => {
                send_telemetry_from_ctx!(TelemetryEvent::AgentModeSetupBannerDismissed, ctx);
                self.remove_agent_setup_speedbump_banner(ctx)
            }
            AgentModeSetupSpeedbumpBannerAction::SetupAgentMode => {
                send_telemetry_from_ctx!(TelemetryEvent::AgentModeSetupBannerAccepted, ctx);
                #[cfg(feature = "local_fs")]
                if let Some(repo_path) = self.current_local_repo_path() {
                    self.mark_agent_init_callout_as_shown_for_directory(repo_path, ctx);
                }
                self.remove_agent_setup_speedbump_banner(ctx);
                self.init_project(false, ctx)
            }
        }
    }

    pub(super) fn codebase_index_speedbump_banner_action(
        &mut self,
        action: CodebaseIndexSpeedbumpBannerAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            CodebaseIndexSpeedbumpBannerAction::ToggleAlwaysAllow => {
                if let Some(banner_state) =
                    &mut self.inline_banners_state.codebase_index_speedbump_banner
                {
                    banner_state.toggle_always_allow_checked();
                }
                ctx.notify();
            }
            CodebaseIndexSpeedbumpBannerAction::AllowIndexing => {
                if let Some(banner_state) =
                    &mut self.inline_banners_state.codebase_index_speedbump_banner
                {
                    // Set "Read files" setting to true if the checkbox was checked
                    if banner_state.always_allow_checked {
                        CodeSettings::handle(ctx).update(ctx, |model, ctx| {
                            report_if_error!(model.auto_indexing_enabled.set_value(true, ctx));
                        });
                    }

                    // Index the codebase
                    CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
                        manager.index_directory(banner_state.repo_path.clone(), ctx);
                    });

                    // Change state to indexing
                    banner_state.show_indexing_banner();
                }
                ctx.notify();
            }
            CodebaseIndexSpeedbumpBannerAction::Close => {
                if let Some(banner_state) = self
                    .inline_banners_state
                    .codebase_index_speedbump_banner
                    .take()
                {
                    // If user dismissed the banner, we want to persist the dismissal (only if it's a speedbump banner).
                    if banner_state.visibility_state == VisibilityState::Speedbump {
                        let mut dismissed_repo_paths = AISettings::as_ref(ctx)
                            .codebase_index_speedbump_banner_dismissed_for_repo_paths
                            .clone();
                        dismissed_repo_paths.push(banner_state.repo_path.clone());
                        AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
                            if let Err(e) = ai_settings
                                .codebase_index_speedbump_banner_dismissed_for_repo_paths
                                .set_value(dismissed_repo_paths, ctx) {
                                    log::error!(
                                        "Failed to persist 'Codebase indexing speedbump banner dismissed' setting: {e}"
                                    );
                                }
                        });
                    }

                    // Remove banner
                    self.model
                        .lock()
                        .block_list_mut()
                        .remove_inline_banner(banner_state.id);
                }
                ctx.notify();
            }
            CodebaseIndexSpeedbumpBannerAction::ViewStatus => {
                ctx.emit(Event::OpenSettings(SettingsSection::CodeIndexing));
            }
            CodebaseIndexSpeedbumpBannerAction::DismissForever => {
                AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
                    if let Err(e) = ai_settings
                        .codebase_index_speedbump_banner_globally_dismissed
                        .set_value(true, ctx)
                    {
                        log::error!(
                            "Failed to persist 'Codebase indexing speedbump banner globally dismissed' setting: {e}"
                        );
                    }
                });
                if let Some(banner_state) = self
                    .inline_banners_state
                    .codebase_index_speedbump_banner
                    .take()
                {
                    self.model
                        .lock()
                        .block_list_mut()
                        .remove_inline_banner(banner_state.id);
                }
                ctx.notify();
            }
        }
    }

    pub(super) fn anonymous_user_ai_sign_up_banner_action(
        &mut self,
        action: AnonymousUserLoginBannerAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            AnonymousUserLoginBannerAction::SignUp => {
                ctx.emit(Event::SignupAnonymousUser {
                    entrypoint: AnonymousUserSignupEntrypoint::LoginGatedFeature,
                });
                self.remove_anonymous_user_ai_sign_up_banner(ctx);
            }
            AnonymousUserLoginBannerAction::Close => {
                self.remove_anonymous_user_ai_sign_up_banner(ctx);
            }
        }
    }

    pub(super) fn insert_anonymous_user_ai_sign_up_banner(&mut self, ctx: &mut ViewContext<Self>) {
        if *GeneralSettings::as_ref(ctx)
            .anonymous_user_ai_sign_up_banner_shown
            .value()
        {
            return;
        }

        let banner_id = self.inline_banners_state.next_banner_id();
        let banner_state = AnonymousUserAISignUpBannerState::new(banner_id);

        self.model
            .lock()
            .block_list_mut()
            .append_inline_banner_with_custom_height(
                InlineBannerItem::new(banner_id, InlineBannerType::AnonymousUserAISignUp),
                3.0,
            );

        self.inline_banners_state.anonymous_user_ai_sign_up_banner = Some(banner_state);
        GeneralSettings::handle(ctx).update(ctx, |settings, ctx| {
            let _ = settings
                .anonymous_user_ai_sign_up_banner_shown
                .set_value(true, ctx);
        });

        ctx.notify();
    }

    pub(super) fn remove_anonymous_user_ai_sign_up_banner(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(banner_state) = self
            .inline_banners_state
            .anonymous_user_ai_sign_up_banner
            .take()
        {
            self.model
                .lock()
                .block_list_mut()
                .remove_inline_banner(banner_state.id);
            ctx.notify();
        }
    }

    pub(super) fn remove_aws_bedrock_login_banner(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(banner_state) = self.inline_banners_state.aws_bedrock_login_banner.take() {
            self.model
                .lock()
                .block_list_mut()
                .remove_inline_banner(banner_state.id);
        }
        ctx.notify();
    }

    pub(super) fn handle_aws_bedrock_login_banner_action(
        &mut self,
        action: AwsBedrockLoginBannerAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            AwsBedrockLoginBannerAction::Login => {
                self.run_aws_login_command(ctx);
            }
            AwsBedrockLoginBannerAction::DontShowAgain => {
                AISettings::handle(ctx).update(ctx, |ai_settings, ctx| {
                    report_if_error!(ai_settings
                        .aws_bedrock_login_banner_dismissed
                        .set_value(true, ctx));
                });
            }
            AwsBedrockLoginBannerAction::Dismiss => {
                // Mark as dismissed for this session (won't reappear until app restart)
                ByoLlmAuthBannerSessionState::handle(ctx).update(ctx, |state, ctx| {
                    state.dismiss(ctx);
                });
            }
        }
        self.remove_aws_bedrock_login_banner(ctx);
    }

    /// Runs the AWS login command configured in settings to refresh Bedrock credentials.
    /// Doing this in PTY vs just a subprocess allows the user to see any output/errors
    /// from the command directly in the terminal. Also, `aws login` commands may require
    /// user interaction (e.g. "do you want to override X profile? y/n" is common)
    pub(super) fn run_aws_login_command(&mut self, ctx: &mut ViewContext<Self>) {
        let login_command = AISettings::as_ref(ctx)
            .aws_bedrock_auth_refresh_command
            .value()
            .clone();

        if login_command.is_empty() {
            log::warn!("AWS login command is not configured");
            return;
        }

        // Track that we're running an AWS login command so we can detect
        // "command not found" if AWS CLI isn't installed
        self.is_pending_aws_login = true;

        // Write the command to the PTY and execute it
        let command_bytes = login_command.into_bytes();
        self.clear_line_editor_and_write_to_pty(command_bytes, ctx);
        self.write_to_pty(vec![escape_sequences::C0::CR], ctx);
    }

    /// Checks if the current model request could be served via AWS Bedrock and the user
    /// isn't already using it. If so, inserts a banner prompting the user to log in.
    ///
    /// The banner is shown when the user could be using AWS Bedrock to save on warp AI spend, but isn't.
    pub(super) fn maybe_insert_aws_bedrock_login_banner(
        &mut self,
        model_id: &LLMId,
        ctx: &mut ViewContext<Self>,
    ) {
        // Don't show if already displayed
        if self.inline_banners_state.aws_bedrock_login_banner.is_some() {
            return;
        }

        // Check if dismissed (either permanently via "Don't show again" or for this session via "X")
        if ByoLlmAuthBannerSessionState::as_ref(ctx).is_dismissed() {
            return;
        }

        // Check if AWS Bedrock is available in the workspace
        if !UserWorkspaces::as_ref(ctx).is_aws_bedrock_credentials_enabled(ctx) {
            return;
        }

        // Check if the model supports AWS Bedrock routing
        let llm_prefs = LLMPreferences::as_ref(ctx);
        let Some(llm_info) = llm_prefs.get_llm_info(model_id) else {
            return;
        };

        let supports_aws_bedrock = llm_info
            .host_configs
            .get(&LLMModelHost::AwsBedrock)
            .is_some_and(|config| config.enabled);
        if !supports_aws_bedrock {
            return;
        }

        if matches!(
            ApiKeyManager::as_ref(ctx).aws_credentials_state(),
            AwsCredentialsState::Loaded { .. }
        ) {
            return;
        }

        // User doesn't have AWS credentials - show the banner
        let banner_id = self.inline_banners_state.next_banner_id();
        self.inline_banners_state.aws_bedrock_login_banner = Some(AwsBedrockLoginBannerState {
            id: banner_id,
            login_button_mouse_state: Default::default(),
            dismiss_button_mouse_state: Default::default(),
            dont_show_again_button_mouse_state: Default::default(),
        });

        self.model
            .lock()
            .block_list_mut()
            .append_inline_banner_with_custom_height(
                InlineBannerItem::new(banner_id, InlineBannerType::AwsBedrockLogin),
                3.5,
            );

        ctx.notify();
    }

    pub(super) fn remove_aws_cli_not_installed_banner(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(banner_state) = self
            .inline_banners_state
            .aws_cli_not_installed_banner
            .take()
        {
            self.model
                .lock()
                .block_list_mut()
                .remove_inline_banner(banner_state.id);
        }
        ctx.notify();
    }

    pub(super) fn handle_aws_cli_not_installed_banner_action(
        &mut self,
        action: AwsCliNotInstalledBannerAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            AwsCliNotInstalledBannerAction::LearnMore => {
                ctx.open_url(AwsCliNotInstalledBannerAction::docs_url());
            }
            AwsCliNotInstalledBannerAction::Dismiss => {}
        }
        self.remove_aws_cli_not_installed_banner(ctx);
    }

    /// Checks if the user tried to run an AWS login command and the AWS CLI wasn't installed.
    /// If so, shows a helpful banner explaining the issue.
    pub(super) fn maybe_show_aws_cli_not_installed_suggestion(
        &mut self,
        exit_code: ExitCode,
        ctx: &mut ViewContext<Self>,
    ) {
        // Check if we were waiting for an AWS login command result
        let was_pending = self.is_pending_aws_login;
        // Always reset the flag
        self.is_pending_aws_login = false;

        if !was_pending {
            return;
        }

        // Check if the command failed with "command not found"
        if !exit_code.was_command_not_found() {
            return;
        }

        // Don't show if already displayed
        if self
            .inline_banners_state
            .aws_cli_not_installed_banner
            .is_some()
        {
            return;
        }

        // Show the banner
        let banner_id = self.inline_banners_state.next_banner_id();
        self.inline_banners_state.aws_cli_not_installed_banner =
            Some(AwsCliNotInstalledBannerState::new(banner_id));

        self.model
            .lock()
            .block_list_mut()
            .append_inline_banner_with_custom_height(
                InlineBannerItem::new(banner_id, InlineBannerType::AwsCliNotInstalled),
                3.5,
            );

        ctx.notify();
    }

    /// Inserts a banner notifying the user that the shell process has terminated.
    pub(super) fn insert_shell_process_terminated_banner(
        &mut self,
        termination_type: shell_terminated_banner::TerminationType,
        ctx: &mut ViewContext<Self>,
    ) {
        // If we successfully bootstrapped, show a simple "Shell exited" banner.
        if self.is_login_shell_bootstrapped {
            let banner_id = self.inline_banners_state.next_banner_id();
            self.inline_banners_state.shell_process_terminated_banner =
                Some(ShellProcessTerminatedBanner {
                    banner_id,
                    was_premature_termination: !self.is_login_shell_bootstrapped,
                });
            // In this case, the active block is actually the last block that was run
            // before exiting; it's not a special hidden block. In other words, the "active" block
            // is read-only and no additional blocks will be added to the block list. That's why
            // we need to explicitly insert _after_ the active block.
            let active_block_index = self.model.lock().block_list().active_block_index();
            self.model
                .lock()
                .block_list_mut()
                .insert_inline_banner_after_block(
                    active_block_index,
                    InlineBannerItem::new(banner_id, InlineBannerType::ShellProcessTerminated),
                );
        } else {
            let (termination_reason, termination_details, exit_reason) = match &termination_type {
                shell_terminated_banner::TerminationType::PtySpawnFailure { .. } => {
                    (Some("PtySpawnFailure".to_string()), None, None)
                }
                shell_terminated_banner::TerminationType::Premature {
                    shell_detail,
                    reason,
                } => (
                    Some("Premature".to_string()),
                    Some(shell_detail.into()),
                    Some(reason),
                ),
                _ => (None, None, None),
            };

            if let Some(termination_reason) = termination_reason {
                let (shell_path, shell_type) = self.get_shell_starter_local(ctx).unzip();
                let antivirus_name = AntivirusInfo::as_ref(ctx).get();

                let long_os_version = crate::system::long_os_version(ctx);

                send_telemetry_from_ctx!(
                    TelemetryEvent::ShellTerminatedPrematurely {
                        shell_type,
                        shell_path,
                        reason: termination_reason,
                        reason_details: termination_details,
                        antivirus_name: antivirus_name.map(ToOwned::to_owned),
                        long_os_version,
                        exit_reason: exit_reason.map(|exit_reason| format!("{exit_reason:?}")),
                    },
                    ctx
                );
            };

            let banner = ctx.add_typed_action_view(|ctx| {
                shell_terminated_banner::ShellTerminatedBanner::new(termination_type, ctx)
            });

            self.insert_rich_content(
                None,
                banner,
                None,
                RichContentInsertionPosition::Append {
                    insert_below_long_running_block: true,
                },
                ctx,
            );
        }

        ctx.notify();
    }

    /// Inserts telemetry policy banner into the blocklist.
    pub fn insert_telemetry_banner(&mut self, is_onboarded: bool, ctx: &mut ViewContext<Self>) {
        // Don't ever show telemetry banner for enterprise users.
        if UserWorkspaces::as_ref(ctx)
            .current_workspace()
            .is_some_and(|w| matches!(w.billing_metadata.customer_type, CustomerType::Enterprise))
        {
            return;
        }

        if FeatureFlag::GlobalAIAnalyticsBanner.is_enabled()
            && !GeneralSettings::as_ref(ctx)
                .telemetry_banner_dismissed
                .value()
            // Do not insert telemetry banner if one is already showing
            // (Happens in the case of a new user going from loginless to login
            // without dismissing banner the first time)
            && !self.rich_content_views.iter().any(|content| content.is_telemetry_banner())
        {
            let banner = ctx.add_view(|ctx| TelemetryBanner::new(is_onboarded, ctx));
            self.insert_rich_content(
                None,
                banner.clone(),
                Some(RichContentMetadata::TelemetryBanner {
                    telemetry_banner_handle: banner,
                }),
                RichContentInsertionPosition::Append {
                    insert_below_long_running_block: true,
                },
                ctx,
            );
            ctx.notify();
        }
    }

    pub(super) fn hide_telemetry_banner_permanently(&mut self, ctx: &mut ViewContext<Self>) {
        GeneralSettings::handle(ctx).update(ctx, |general_settings, ctx| {
            let _ = general_settings
                .telemetry_banner_dismissed
                .set_value(true, ctx);
        });
        for rich_content in self.rich_content_views.iter() {
            if let Some(RichContentMetadata::TelemetryBanner {
                telemetry_banner_handle,
            }) = rich_content.metadata()
            {
                self.model
                    .lock()
                    .block_list_mut()
                    .remove_rich_content(telemetry_banner_handle.id());
            }
        }
        ctx.notify();
    }

    /// Redetermine focus in the terminal view -- note that this will not steal focus
    /// from other parts of the app, the find bar, or the block filter editor.
    ///
    /// See [`Self::redetermine_global_focus`] to change focus without checking that the terminal is focused.
    pub(super) fn redetermine_terminal_focus(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        // Only reset the focus if this terminal is currently focused, don't steal it from
        // another part of the app
        let reset_focus = ctx.is_self_or_child_focused()
            && !self.find_bar.is_self_or_child_focused(ctx)
            && !self.block_filter_editor.is_self_or_child_focused(ctx);
        if reset_focus {
            self.redetermine_global_focus(ctx);
        }

        reset_focus
    }

    /// Recomputes the chip values for the Warp prompt (i.e. _not_ PS1).
    pub(super) fn refresh_warp_prompt(&mut self, ctx: &mut ViewContext<Self>) {
        // Ask the per-repo sub-model to re-fetch metadata so the chip values
        // reflect the latest git state (branch, diff stats, etc.).
        #[cfg(feature = "local_fs")]
        if let Some(handle) = &self.git_repo_status {
            handle.update(ctx, |model, ctx| {
                model.refresh_metadata(ctx);
            });
        }

        self.input.update(ctx, |input, ctx| {
            input.update_prompt_display_chips(ctx);
        });

        self.current_prompt.update(ctx, |prompt_type, ctx| {
            if let PromptType::Dynamic { prompt } = prompt_type {
                prompt.update(ctx, |current_prompt, ctx| {
                    current_prompt
                        .update_context(self.model.lock().block_list().active_block(), ctx);
                });
            }
        });
    }

    pub fn current_state(&self) -> TerminalViewStateChange {
        self.current_state
    }

    pub(super) fn set_current_state(
        &mut self,
        new_state: TerminalViewState,
        ctx: &mut ViewContext<Self>,
    ) {
        self.current_state = TerminalViewStateChange {
            state: new_state,
            timestamp: Instant::now(),
        };

        ctx.emit(Event::TerminalViewStateChanged);

        // Notify pane header to re-render (error indicator may change).
        self.pane_configuration.update(ctx, |config, ctx| {
            config.notify_header_content_changed(ctx);
        });
    }

    pub(super) fn maybe_emit_terminal_view_state_changed_for_long_running_block(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.did_notify_long_running || !self.is_long_running() {
            return;
        }

        self.did_notify_long_running = true;
        ctx.emit(Event::TerminalViewStateChanged);
        self.update_pane_configuration(ctx);

        // Redetermine focus when the block becomes long-running. This recovers focus for
        // queued commands: when the previous block completes, focus moves to the input box
        // (because no new block is live yet), and nothing moves it back once the queued
        // block starts. By the time we arrive here `is_active_and_long_running()` is true,
        // so `redetermine_global_focus` correctly returns focus to the terminal view.
        //
        // Skip this pre-bootstrap: long-running pre-bootstrap blocks (e.g. a `.zshrc` that
        // issues a `read` prompt) need the input box to remain focused so the user can type
        // a response to unblock bootstrap.
        if self.model.lock().block_list().is_bootstrapped() {
            self.redetermine_terminal_focus(ctx);
        }
    }
}
