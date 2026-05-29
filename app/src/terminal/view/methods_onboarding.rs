use super::*;

impl TerminalView {
    pub(super) fn start_agent_onboarding_tutorial(
        &mut self,
        version: AgentOnboardingVersion,
        ctx: &mut ViewContext<Self>,
    ) {
        // If we are already showing the onboarding callout, do nothing.
        if self.onboarding_callout_view.is_some() {
            log::warn!("Attempted to start onboarding tutorial when one is already active.");
            return;
        }

        // The first Agent Modality callout expects terminal mode. If the default
        // session mode is Agent (e.g. from cloud-synced settings), the tab
        // may already be in agent view — exit it first.
        self.exit_agent_view(ctx);

        // Remove the terminal zero-state welcome block so it doesn't appear
        // underneath the onboarding callout.
        let zero_state_ids: Vec<_> = self
            .rich_content_views
            .iter()
            .filter(|view| {
                matches!(
                    view.metadata(),
                    Some(RichContentMetadata::TerminalViewZeroState)
                )
            })
            .map(|view| view.view_id())
            .collect();
        for view_id in zero_state_ids {
            self.model
                .lock()
                .block_list_mut()
                .remove_rich_content(view_id);
            self.rich_content_views
                .retain(|view| view.view_id() != view_id);
        }

        log::info!("Starting onboarding tutorial with version: {:?}", version);

        let view = ctx.add_typed_action_view(|ctx| {
            let keybindings = build_onboarding_keybindings(ctx);

            match version {
                AgentOnboardingVersion::UniversalInput { has_project } => {
                    let initial_natural_language_detection_enabled = AISettings::handle(ctx)
                        .as_ref(ctx)
                        .is_nld_in_terminal_enabled(ctx);
                    OnboardingCalloutView::new_universal_input(
                        has_project,
                        initial_natural_language_detection_enabled,
                        keybindings,
                        ctx,
                    )
                }
                AgentOnboardingVersion::AgentModality {
                    has_project,
                    intention,
                } => {
                    let initial_natural_language_detection_enabled = AISettings::handle(ctx)
                        .as_ref(ctx)
                        .is_nld_in_terminal_enabled(ctx);
                    OnboardingCalloutView::new_agent_modality(
                        has_project,
                        intention,
                        initial_natural_language_detection_enabled,
                        keybindings,
                        ctx,
                    )
                }
            }
        });

        ctx.subscribe_to_view(&view, |me, callout_view, event, ctx| {
            me.handle_onboarding_callout_view_event(&callout_view, event, ctx)
        });

        view.update(ctx, |view, ctx| {
            view.start_onboarding(ctx);
        });

        self.onboarding_callout_view = Some(view);
        ctx.notify();
    }

    pub(super) fn handle_onboarding_callout_view_event(
        &mut self,
        callout_view: &ViewHandle<OnboardingCalloutView>,
        event: &OnboardingCalloutViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            OnboardingCalloutViewEvent::Completed {
                final_state: FinalState::Submit,
            } => {
                // Submit whatever is currently in the input as an Agent Mode query.
                // We explicitly override any Shell lock so this always routes to AI.
                let prompt = callout_view.as_ref(ctx).prompt_string(ctx);

                if FeatureFlag::AgentView.is_enabled() {
                    self.enter_agent_view_for_new_conversation(
                        Some(prompt),
                        AgentViewEntryOrigin::OnboardingCallout,
                        ctx,
                    )
                } else {
                    self.set_ai_input_mode_with_query(Some(&prompt), ctx);
                    self.input()
                        .update(ctx, |input, ctx| input.input_enter(ctx));
                }

                self.onboarding_callout_view = None;
                ctx.emit(Event::OnboardingTutorialCompleted);
                ctx.notify();
            }
            OnboardingCalloutViewEvent::Completed {
                final_state: FinalState::Initialize,
            } => {
                // Clear the input first, then submit the initialization query
                self.input
                    .update(ctx, |input, ctx| input.replace_buffer_content("", ctx));
                // Submit /init as an Agent Mode query
                self.enter_agent_view_for_new_conversation(
                    Some("/init".to_string()),
                    AgentViewEntryOrigin::Onboarding,
                    ctx,
                );
                self.onboarding_callout_view = None;
                ctx.emit(Event::OnboardingTutorialCompleted);
                ctx.notify();
            }
            OnboardingCalloutViewEvent::Completed {
                final_state: FinalState::Skip | FinalState::Finish,
            } => {
                // Close the callout without submitting and clear the input.
                self.input
                    .update(ctx, |input, ctx| input.replace_buffer_content("", ctx));
                self.onboarding_callout_view = None;
                ctx.emit(Event::OnboardingTutorialCompleted);
                ctx.notify();
            }
            OnboardingCalloutViewEvent::Completed {
                final_state: FinalState::BackToTerminal,
            } => {
                // Exit the agent view and return to terminal
                self.exit_agent_view(ctx);
                self.input
                    .update(ctx, |input, ctx| input.replace_buffer_content("", ctx));
                self.onboarding_callout_view = None;
                ctx.emit(Event::OnboardingTutorialCompleted);
                ctx.notify();
            }
            OnboardingCalloutViewEvent::StateUpdated => {
                self.apply_onboarding_callout_query_to_input(callout_view, ctx);
                ctx.notify();
            }
            OnboardingCalloutViewEvent::EnterAgentModality => {
                // Enter agent view without submitting a prompt (mid-flow entry)
                self.enter_agent_view_for_new_conversation(
                    None,
                    AgentViewEntryOrigin::Onboarding,
                    ctx,
                );
                // Re-focus the callout so its keybindings continue to work
                self.focus_onboarding_callout_if_active(ctx);
                ctx.notify();
            }
            OnboardingCalloutViewEvent::NaturalLanguageDetectionToggled(enabled) => {
                // Apply the setting immediately when the user toggles the checkbox
                self.apply_natural_language_detection_setting(*enabled, ctx);
            }
        }
    }

    pub(super) fn apply_natural_language_detection_setting(
        &mut self,
        enable: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings
                .nld_in_terminal_enabled_internal
                .set_value(enable, ctx));
        });
    }

    pub(super) fn maybe_render_onboarding_callout(
        &self,
        menu_positioning: MenuPositioning,
        should_position_above_zero_state: bool,
        stack: &mut Stack,
        app: &AppContext,
    ) {
        let Some(onboarding_view) = self.onboarding_callout_view.as_ref() else {
            return;
        };

        let (position_id, anchor, child_anchor, offset) = match (
            should_position_above_zero_state,
            self.agent_view_zero_state_save_position_id(app),
            menu_positioning,
        ) {
            (true, Some(zero_state_position_id), _) => (
                zero_state_position_id,
                PositionedElementAnchor::TopLeft,
                ChildAnchor::BottomLeft,
                vec2f(4., -8.),
            ),
            (_, _, MenuPositioning::BelowInputBox) => (
                self.input.as_ref(app).status_free_input_save_position_id(),
                PositionedElementAnchor::BottomLeft,
                ChildAnchor::TopLeft,
                vec2f(4., 8.),
            ),
            (_, _, MenuPositioning::AboveInputBox) => (
                self.input.as_ref(app).status_free_input_save_position_id(),
                PositionedElementAnchor::TopLeft,
                ChildAnchor::BottomLeft,
                vec2f(4., -8.),
            ),
        };

        stack.add_positioned_overlay_child(
            ChildView::new(onboarding_view).finish(),
            OffsetPositioning::offset_from_save_position_element(
                position_id.as_str(),
                offset,
                PositionedElementOffsetBounds::WindowByPosition,
                anchor,
                child_anchor,
            ),
        );
    }

    // Read the current terminal input text from the onboarding tutorial callout
    // and apply it to the terminal input box. Lock the input mode based on query type.
    pub(super) fn apply_onboarding_callout_query_to_input(
        &mut self,
        callout_view: &ViewHandle<OnboardingCalloutView>,
        ctx: &mut ViewContext<Self>,
    ) {
        let prompt = callout_view.as_ref(ctx).prompt(ctx);

        if let OnboardingQuery::None = prompt {
            // No-op: don't clear existing input
            return;
        }

        self.input.update(ctx, |input, ctx| {
            match &prompt {
                OnboardingQuery::TerminalCommand(text) => {
                    input.replace_buffer_content(text, ctx);
                }
                OnboardingQuery::AgentPrompt(text) => {
                    input.replace_buffer_content(text, ctx);
                    // Force agent mode, overriding any shell lock
                    input.ensure_agent_mode_for_ai_features(
                        true,
                        Some(InputTypeAutoDetectionSource::OnboardingAgentPrompt),
                        ctx,
                    );
                }
                _ => {}
            }
        });

        ctx.focus(callout_view);
    }
}
