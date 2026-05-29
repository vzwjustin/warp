use super::*;

impl TerminalView {
    fn render_filter_element(
        block_index: BlockIndex,
        active_filter_editor_block_index: Option<BlockIndex>,
        filter_mouse_state: MouseStateHandle,
        has_active_filter: bool,
        tool_tip_below_button: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon = Container::new(
            ConstrainedBox::new(if has_active_filter {
                icons::Icon::FilterFunnelFilled
                    .to_warpui_icon(appearance.theme().accent())
                    .finish()
            } else {
                icons::Icon::FilterFunnel
                    .to_warpui_icon(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_2()),
                    )
                    .finish()
            })
            .with_height(26.)
            .with_width(26.)
            .finish(),
        );

        let should_disable_filter_button =
            active_filter_editor_block_index.is_some_and(|active_filter_editor_block_index| {
                block_index == active_filter_editor_block_index
            });

        SavePosition::new(
            render_hoverable_block_button(
                icon,
                Some(ToolbeltButtonTooltip {
                    label: "Filter block output".to_string(),
                    tool_tip_below_button,
                }),
                should_disable_filter_button,
                true,
                filter_mouse_state,
                appearance.theme(),
                appearance.ui_builder(),
                move |ctx, _, _| {
                    ctx.dispatch_typed_action(TerminalAction::OpenBlockFilterEditor(block_index))
                },
            ),
            filter_button_position_id(block_index).as_str(),
        )
        .finish()
    }

    fn render_bookmark_element(
        index: BlockIndex,
        bookmark_mouse_state: MouseStateHandle,
        is_bookmarked: bool,
        tool_tip_below_button: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let bookmark_fill_color: ColorU = theme.accent().into();
        let icon_color = if is_bookmarked {
            bookmark_fill_color
        } else {
            theme.sub_text_color(theme.surface_2()).into()
        };

        let icon_path = if is_bookmarked {
            "bundled/svg/bookmark_filled.svg"
        } else {
            "bundled/svg/bookmark.svg"
        };

        let icon = Container::new(
            ConstrainedBox::new(Icon::new(icon_path, icon_color).finish())
                .with_height(26.)
                .with_width(26.)
                .finish(),
        );

        render_hoverable_block_button(
            icon,
            Some(ToolbeltButtonTooltip {
                label: "Bookmark this block to quickly scroll to it".to_string(),
                tool_tip_below_button,
            }),
            false,
            true,
            bookmark_mouse_state,
            theme,
            appearance.ui_builder(),
            move |ctx, _, _| {
                ctx.dispatch_typed_action(TerminalAction::BookmarkBlock(index));
            },
        )
    }

    pub(super) fn is_jump_to_bottom_of_block_element_hovered(&self) -> bool {
        self.mouse_states
            .jump_to_bottom_of_block_button
            .lock()
            .is_ok_and(|handle| handle.is_hovered())
    }

    pub(super) fn render_jump_to_bottom_of_block_element(
        &self,
        overhanging_block: OverhangingBlock,
        is_long_running_command: bool,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        Hoverable::new(
            self.mouse_states.jump_to_bottom_of_block_button.clone(),
            move |state| {
                let icon_color: ColorU = theme.sub_text_color(theme.surface_2()).into();
                let icon_path = "bundled/svg/vertical_align_bottom.svg";

                let container = Container::new(
                    ConstrainedBox::new(Icon::new(icon_path, icon_color).finish())
                        .with_height(JUMP_TO_BOTTOM_OF_BLOCK_ICON_SIZE_PX.as_f32())
                        .with_width(JUMP_TO_BOTTOM_OF_BLOCK_ICON_SIZE_PX.as_f32())
                        .finish(),
                )
                .with_uniform_padding(JUMP_TO_BOTTOM_OF_BLOCK_BUTTON_PADDING_PX.as_f32())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    JUMP_TO_BOTTOM_OF_BLOCK_CORNER_RADIUS_PX.as_f32(),
                )));

                let container = if state.is_hovered() || state.is_clicked() {
                    container.with_background(theme.surface_2())
                } else {
                    container
                };

                let mut stack = Stack::new().with_child(container.finish());

                if state.is_hovered() {
                    let input_mode = *InputModeSettings::as_ref(app).input_mode.value();
                    let tool_tip_text = if overhanging_block.is_most_recent_block()
                        && input_mode.is_inverted_blocklist()
                        && is_long_running_command
                    {
                        "Lock scrolling at bottom of block".to_string()
                    } else {
                        "Jump to the bottom of this block".to_string()
                    };

                    let tool_tip = appearance
                        .ui_builder()
                        .tool_tip(tool_tip_text)
                        .build()
                        .finish();

                    stack.add_positioned_child(
                        tool_tip,
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., JUMP_TO_BOTTOM_OF_BLOCK_TOOLTIP_OFFSET_Y_PX.as_f32()),
                            ParentOffsetBounds::Unbounded,
                            ParentAnchor::TopRight,
                            ChildAnchor::BottomRight,
                        ),
                    );
                }

                stack.finish()
            },
        )
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(TerminalAction::ScrollToBottomOfOverhangingBlock(
                overhanging_block,
            ));
        })
        .on_hover(move |mouse_in, ctx, _, position| {
            // Since this element is on top of the block list element, we need to start a block hover here
            // rather than relying on the block list element itself to manage hover state.
            if mouse_in {
                ctx.dispatch_typed_action(TerminalAction::BlockHover(BlockHoverAction::Begin {
                    position,
                    block_index: overhanging_block.block_index(),
                }));
            } else {
                ctx.dispatch_typed_action(TerminalAction::BlockHover(BlockHoverAction::Clear));
            }
        })
        .finish()
    }

    fn render_label_element(
        index: BlockIndex,
        model: &TerminalModel,
        mouse_state: Option<&MouseStateHandle>,
        sessions: &Sessions,
        padding_x: Pixels,
        tool_tip_below_button: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let terminal_theme_prompt: ColorU = appearance
            .theme()
            .sub_text_color(appearance.theme().background())
            .into();

        let prompt = Text::new_inline(
            Self::block_prompt(model, sessions, index),
            appearance.monospace_font_family(),
            appearance.monospace_font_size() * WARP_PROMPT_HEIGHT_LINES,
        )
        .with_style(Properties::default().weight(appearance.monospace_font_weight()))
        .with_color(terminal_theme_prompt)
        .finish();

        let mut label_row = Flex::row().with_child(prompt);

        let is_live = Self::is_block_duration_live(model, index);
        if let Some(duration_string) = Self::block_duration_text(model, index) {
            let duration = Text::new_inline(
                duration_string,
                appearance.monospace_font_family(),
                appearance.monospace_font_size() * WARP_PROMPT_HEIGHT_LINES,
            )
            .with_style(Properties::default().weight(appearance.monospace_font_weight()))
            .with_color(terminal_theme_prompt)
            .finish();

            // Wrap in LiveElement to trigger periodic repaints while the command
            // is still running, so the counter updates live.
            let duration: Box<dyn Element> = if is_live {
                LiveElement::new(duration, LIVE_COMMAND_DURATION_REPAINT_INTERVAL).finish()
            } else {
                duration
            };

            label_row.add_child(if let Some(state) = mouse_state {
                Hoverable::new(state.clone(), |state| {
                    let mut stack = Stack::new().with_child(duration);
                    if state.is_hovered() {
                        let tool_tip = appearance
                            .ui_builder()
                            .tool_tip(Self::block_start_and_completed_ts(model, index))
                            .build()
                            .finish();
                        if tool_tip_below_button {
                            stack.add_positioned_child(
                                tool_tip,
                                OffsetPositioning::offset_from_parent(
                                    Vector2F::new(30., 5.),
                                    ParentOffsetBounds::ParentByPosition,
                                    ParentAnchor::BottomMiddle,
                                    ChildAnchor::TopMiddle,
                                ),
                            );
                        } else {
                            stack.add_positioned_child(
                                tool_tip,
                                OffsetPositioning::offset_from_parent(
                                    Vector2F::new(30., -5.),
                                    ParentOffsetBounds::ParentByPosition,
                                    ParentAnchor::TopMiddle,
                                    ChildAnchor::BottomMiddle,
                                ),
                            );
                        }
                    }
                    stack.finish()
                })
                .with_hover_in_delay(Duration::from_millis(500))
                .finish()
            } else {
                duration
            });
        } else if Self::is_block_executing(model, index) {
            // Block is executing but less than 1 second has elapsed — no duration
            // text to show yet. Add an invisible LiveElement to kick off the
            // repaint timer so the counter appears as soon as 1s elapses.
            label_row.add_child(
                LiveElement::new(
                    ConstrainedBox::new(Empty::new().finish())
                        .with_width(0.)
                        .with_height(0.)
                        .finish(),
                    LIVE_COMMAND_DURATION_REPAINT_INTERVAL,
                )
                .finish(),
            );
        }

        SavePosition::new(
            Container::new(label_row.finish())
                .with_padding_left(padding_x.as_f32())
                .with_padding_right(padding_x.as_f32())
                .with_padding_bottom(16.)
                .finish(),
            format!("block_index:{index}").as_str(),
        )
        .finish()
    }

    pub(super) fn render_input(&self) -> Box<dyn Element> {
        let input = ChildView::new(&self.input).finish();
        Hoverable::new(self.input_hoverable_handle.clone(), |_| input)
            // We rely on the hover-out delay for the "Request edit access"
            // button UX for shared sessions.
            .with_hover_out_delay(Duration::from_millis(500))
            .finish()
    }

    fn render_inline_banners(
        &self,
        appearance: &Appearance,
        app: &AppContext,
        model: &TerminalModel,
    ) -> HashMap<usize, Box<dyn Element>> {
        let mut inline_banners = HashMap::new();

        // If the notifications discovery banner is open, render it.
        if let NotificationsDiscoveryBanner::Open {
            trigger,
            request_outcome,
            state,
        } = &self.inline_banners_state.notifications_discovery_banner
        {
            inline_banners.insert(
                state.banner_id,
                render_inline_notifications_discovery_banner(
                    *trigger,
                    request_outcome.clone(),
                    state,
                    SessionSettings::as_ref(app).notifications.mode,
                    appearance,
                ),
            );
        }

        // If the notifications error banner is open, render it.
        if let NotificationsErrorBannerType::Open { state } = &self
            .inline_banners_state
            .notifications_error_banner
            .banner_type
        {
            let banner_title = self
                .inline_banners_state
                .notifications_error_banner
                .error
                .as_ref()
                .map(|e| e.notifications_error_banner_title())
                .unwrap_or("Error sending notification");

            inline_banners.insert(
                state.banner_id,
                render_inline_notifications_error_banner(
                    banner_title,
                    state,
                    &self.inline_banners_state.notifications_error_banner.error,
                    appearance,
                ),
            );
        }

        for (banner_id, state) in &self.inline_banners_state.ssh_banners {
            inline_banners.insert(
                *banner_id,
                render_inline_ssh_wrapper_banner(state, appearance),
            );
        }

        if let AliasExpansionBanner::Open { state } =
            &self.inline_banners_state.alias_expansion_banner
        {
            inline_banners.insert(state.id, render_alias_expansion_banner(state, appearance));
        }

        if let Some(ShellProcessTerminatedBanner {
            banner_id,
            was_premature_termination,
        }) = self.inline_banners_state.shell_process_terminated_banner
        {
            inline_banners.insert(
                banner_id,
                render_shell_process_terminated_banner(appearance, was_premature_termination),
            );
        }

        if (FeatureFlag::CreatingSharedSessions.is_enabled()
            && ContextFlag::CreateSharedSession.is_enabled())
            || FeatureFlag::ViewingSharedSessions.is_enabled()
        {
            let is_shared_ambient_agent_session = model.is_shared_ambient_agent_session();
            match &self.inline_banners_state.shared_session_banner_state {
                SharedSessionBanners::ActiveShare {
                    started_banner_id,
                    started_at,
                    is_remote_control,
                } => {
                    inline_banners.insert(
                        *started_banner_id,
                        render_inline_shared_session_started_banner(
                            true,
                            is_shared_ambient_agent_session,
                            *is_remote_control,
                            *started_at,
                            appearance,
                        ),
                    );
                }
                SharedSessionBanners::LastShared {
                    started_at,
                    ended_at,
                    started_banner_id,
                    ended_banner_id,
                    is_remote_control,
                } => {
                    inline_banners.insert(
                        *started_banner_id,
                        render_inline_shared_session_started_banner(
                            false,
                            is_shared_ambient_agent_session,
                            *is_remote_control,
                            *started_at,
                            appearance,
                        ),
                    );
                    inline_banners.insert(
                        *ended_banner_id,
                        render_inline_shared_session_ended_banner(
                            is_shared_ambient_agent_session,
                            *is_remote_control,
                            *ended_at,
                            appearance,
                        ),
                    );
                }
                SharedSessionBanners::None => {}
            }
        }

        if let Some(open_in_warp_banner) = &self.inline_banners_state.open_in_warp_banner {
            inline_banners.insert(
                open_in_warp_banner.id,
                render_open_in_warp_banner(open_in_warp_banner, self.view_id, appearance),
            );
        }

        if let Some(vim_banner_state) = &self.inline_banners_state.vim_banner_state {
            inline_banners.insert(
                vim_banner_state.id,
                render_vim_mode_banner(vim_banner_state, appearance),
            );
        }

        if let Some(banner_state) = &self.inline_banners_state.codebase_index_speedbump_banner {
            inline_banners.insert(
                banner_state.id,
                banner_state.render_codebase_index_speedbump_banner(appearance),
            );
        }

        if let Some(banner_state) = &self.inline_banners_state.agent_setup_speedbump_banner {
            inline_banners.insert(
                banner_state.id,
                render_agent_mode_setup_banner(banner_state, appearance),
            );
        }

        if let Some(banner_state) = &self.inline_banners_state.anonymous_user_ai_sign_up_banner {
            inline_banners.insert(banner_state.id, banner_state.render(appearance));
        }

        if let Some(banner_state) = &self.inline_banners_state.aws_bedrock_login_banner {
            inline_banners.insert(
                banner_state.id,
                render_aws_bedrock_login_banner(banner_state, appearance),
            );
        }

        if let Some(banner_state) = &self.inline_banners_state.aws_cli_not_installed_banner {
            inline_banners.insert(
                banner_state.id,
                render_aws_cli_not_installed_banner(banner_state, appearance),
            );
        }

        inline_banners
    }

    pub(super) fn render_alt_screen_element(
        &self,
        app: &AppContext,
        model: &TerminalModel,
        selection_range: Option<ExpandedSelectionRange<Point>>,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // For the alt-screen in a shared session viewer, we need to use
        // the sharer's size exactly. We don't want to render an alt-screen
        // larger than the sharer's since that would look janky.
        // TODO: we should have more ergonomic ways of getting Viewer / Sharer from the session.
        let (rows, columns) = if let Some(Viewer { sharer_size, .. }) = self.shared_session_viewer()
        {
            sharer_size
                .map(|s| (s.num_rows, s.num_cols))
                .unwrap_or((self.size_info.rows(), self.size_info.columns()))
        } else {
            (self.size_info.rows(), self.size_info.columns())
        };

        // Note: The Alt screen relies on the accuracy of the `padding` elements of SizeInfo
        // for things like hit detection and selection. Since we are taking into account the
        // padding separately (using `Align` and `ConstrainedBox`), we need to create a new
        // SizeInfo that reflects the lack of padding on the AltScreenElement directly
        let render_context = self.get_terminal_view_render_context(model, app);

        let enforce_minimum_contrast = *FontSettings::as_ref(app).enforce_minimum_contrast;
        let active_cli_subagent_view = model
            .block_list()
            .active_block()
            .is_agent_in_control()
            .then(|| self.cli_subagent_views.get(model.active_block_id()))
            .flatten();
        let mut alt_screen_element = AltScreenElement::new(
            self.model.clone(),
            render_context,
            self.find_model.clone(),
            enforce_minimum_contrast,
            selection_range.map(|selection| match selection {
                ExpandedSelectionRange::Rect { rows } => rows.mapped(|(start, end)| start..end),
                ExpandedSelectionRange::Regular { start, end, .. } => vec1![start..end],
            }),
            appearance,
            self.alt_screen_scroll_top,
            // TODO(zachbai): Remove this.
            None,
            active_cli_subagent_view.map(|view| ChildView::new(view).finish()),
        );
        if should_use_ligature_rendering(app) {
            alt_screen_element = alt_screen_element.with_ligature_rendering();
        }
        if self.should_hide_cli_agent_cursor_cell(app) {
            alt_screen_element = alt_screen_element.with_hide_cursor_cell();
        }
        alt_screen_element =
            alt_screen_element.with_shared_session_presence(self.shared_session_presence_manager());

        // Pass voice input toggle key if the CLI agent footer should be rendered
        #[cfg(feature = "voice_input")]
        if self.should_render_use_agent_footer(model, app)
            && self.use_agent_footer.as_ref(app).has_cli_agent(app)
        {
            let voice_key = AISettings::as_ref(app)
                .voice_input_toggle_key
                .value()
                .to_key_code();
            alt_screen_element = alt_screen_element.with_voice_input_toggle_key(voice_key);
        }

        let required_terminal_height = self.size_info.cell_height_px.as_f32() * (rows as f32)
            + 2. * self.size_info.padding_y_px().as_f32();
        let pane_height = self.content_element_height_px(app);

        let required_terminal_width = self.size_info.cell_width_px.as_f32() * (columns as f32)
            + 2. * self.size_info.padding_x_px().as_f32();
        let pane_width = self.content_element_width_px(app);

        // If this is a shared session viewer and the height required to display the entire
        // terminal is larger than the height of the pane, we should make it vertically scrollable.
        let should_be_vertical_scrollable = model.shared_session_status().is_active_viewer()
            && required_terminal_height > pane_height;

        // If this is a shared session viewer and the width required to display the entire
        // terminal is larger than the width of the pane, we should make it horizontally scrollable.
        let should_be_horizontal_scrollable = FeatureFlag::ViewingSharedSessions.is_enabled()
            && model.shared_session_status().is_active_viewer()
            && required_terminal_width > pane_width;

        let theme = appearance.theme();
        let element = maybe_wrap_terminal_element_in_scrollable(
            should_be_vertical_scrollable,
            should_be_horizontal_scrollable,
            self.alt_screen_vertical_scroll_state.clone(),
            self.horizontal_clipped_scroll_state.clone(),
            required_terminal_width,
            theme,
            alt_screen_element,
        );

        SavePosition::new(
            Container::new(
                Align::new(
                    ConstrainedBox::new(
                        // We wrap in a `Clipped` to prevent grid text from partially bleeding into the pane header.
                        // This is different from a ClippedScrollable because the alt screen is not actually rendering
                        // unnecessary rows.
                        Clipped::new(element).finish(),
                    )
                    .finish(),
                )
                // Pin the alt-screen origin to the top-left of the pane (adjusted for padding)
                // to prevent a wiggle-like effect when resizing the pane.
                .top_left()
                .finish(),
            )
            .with_vertical_padding(self.size_info.padding_y_px().as_f32())
            .finish(),
            &self.content_element_position_id,
        )
        .finish()
    }

    pub(super) fn render_viewer_loading(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let color = appearance
            .theme()
            .sub_text_color(appearance.theme().background());

        SavePosition::new(
            Align::new(
                Flex::column()
                    .with_child(
                        ConstrainedBox::new(Icon::new("bundled/svg/refresh.svg", color).finish())
                            .with_height(16.)
                            .with_width(16.)
                            .finish(),
                    )
                    .with_child(
                        Text::new_inline("Loading session...", appearance.ui_font_family(), 14.)
                            .with_color(color.into())
                            .finish(),
                    )
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish(),
            )
            .finish(),
            &self.content_element_position_id,
        )
        .finish()
    }

    /// Returns true when cursor rendering should be suppressed because the
    /// CLI agent rich input is open.
    fn should_hide_cli_agent_cursor_cell(&self, app: &AppContext) -> bool {
        CLIAgentSessionsModel::as_ref(app)
            .session(self.view_id)
            .is_some_and(|s| matches!(s.input_state, CLIAgentInputState::Open { .. }))
    }

    pub(super) fn render_block_list_element(
        &self,
        model: &TerminalModel,
        input_mode: InputMode,
        is_scrollable: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let padding_x = self.size_info.padding_x_px;
        let sessions = self.sessions.clone();

        let inline_banners = self.render_inline_banners(appearance, app, model);

        let mut subshell_separators = HashMap::new();

        for (id, command) in self.warpify_state.get_subshell_separators() {
            subshell_separators.insert(*id, render_subshell_separator(command.clone(), appearance));
        }

        // Currently, it is assumed that only the active block can have a block banner, which
        // implies that there can only be one at a time. This assumption can be relaxed once we
        // have an actual use case for that.
        let block_banner = model
            .block_list()
            .active_block()
            .block_banner()
            .map(|banner| match banner {
                WithinBlockBanner::WarpifyBanner(state) => {
                    render_warpification_banner(state, appearance, app)
                }
            });

        let bookmarked_blocks: HashSet<_> = self.bookmarked_blocks.keys().copied().collect();
        let filtered_blocks: HashSet<_> = model.block_list().filtered_blocks();

        let snackbar_header_state = SnackbarHeaderState {
            snackbar_enabled: *BlockListSettings::as_ref(app).snackbar_enabled,
            show_snackbar: self.show_snackbar,
            hover_near_snackbar_area: self.hover_near_snackbar_area,
            state_handle: self.snackbar_header_state.state_handle.clone(),
        };

        let semantic_selection = SemanticSelection::as_ref(app);
        let selection_range = model
            .block_list()
            .renderable_selection(semantic_selection, input_mode.is_inverted_blocklist());

        let terminal_spacing =
            TerminalSettings::as_ref(app).terminal_spacing(appearance.line_height_ratio(), app);

        let enforce_minimum_contrast = *FontSettings::as_ref(app).enforce_minimum_contrast;

        let mut element = BlockListElement::new(
            self.model.clone(),
            self.find_model.clone(),
            input_mode,
            self.get_terminal_view_render_context(model, app),
            self.block_list_mouse_states.clone(),
            snackbar_header_state,
            &terminal_spacing,
            enforce_minimum_contrast,
            appearance,
            Box::new(move |range, label_mouse_states, model, app| {
                range
                    .iter()
                    .enumerate()
                    .map(|(i, index)| {
                        let mut label = Self::render_label_element(
                            *index,
                            model,
                            label_mouse_states.get(index),
                            sessions.as_ref(app),
                            padding_x,
                            i == 0,
                            Appearance::as_ref(app),
                        );
                        // Special-case the last block so there is a reliable way to target it
                        // regardless of the length of the list.
                        if i == range.len() - 1 {
                            label = SavePosition::new(label, "block_index:last").finish()
                        }
                        label
                    })
                    .collect()
            }),
            Box::new(move |range, hovered_index, mouse_states, app| {
                range
                    .iter()
                    .enumerate()
                    .map(|(i, block_index)| {
                        let mouse_state = mouse_states.get(block_index)?.clone();
                        let is_bookmarked = bookmarked_blocks.contains(block_index);

                        if is_bookmarked || hovered_index == Some(*block_index) {
                            Some(Self::render_bookmark_element(
                                *block_index,
                                mouse_state,
                                is_bookmarked,
                                i == 0,
                                Appearance::as_ref(app),
                            ))
                        } else {
                            None
                        }
                    })
                    .collect()
            }),
            Box::new(
                move |range,
                      hovered_index,
                      active_filter_editor_block_index,
                      filtered_blocks,
                      mouse_states,
                      app| {
                    range
                        .iter()
                        .enumerate()
                        .map(|(i, block_index)| {
                            let mouse_state = mouse_states.get(block_index)?.clone();
                            let has_active_filter =
                                filtered_blocks.is_some_and(|filtered_blocks| {
                                    filtered_blocks.contains(block_index)
                                });
                            if has_active_filter
                                || hovered_index == Some(*block_index)
                                || active_filter_editor_block_index == Some(*block_index)
                            {
                                Some(Self::render_filter_element(
                                    *block_index,
                                    active_filter_editor_block_index,
                                    mouse_state,
                                    has_active_filter,
                                    i == 0,
                                    Appearance::as_ref(app),
                                ))
                            } else {
                                None
                            }
                        })
                        .collect()
                },
            ),
            inline_banners,
            subshell_separators,
            HashMap::from_iter(
                self.cli_subagent_views
                    .iter()
                    .map(|(id, view)| (id.clone(), ChildView::new(view).finish())),
            ),
            selection_range,
            block_banner,
            self.inline_banners_state.shared_session_banner_state,
            self.input_size_at_last_frame(app).unwrap_or_default(),
            self.inline_menu_positioner.clone(),
            None,
        );

        if should_use_ligature_rendering(app) {
            element = element.with_ligature_rendering();
        }

        if self.should_hide_cli_agent_cursor_cell(app) {
            element = element.with_hide_cursor_cell();
        }

        // Pass voice input toggle key if the CLI agent footer should be rendered
        #[cfg(feature = "voice_input")]
        if self.should_render_use_agent_footer(model, app)
            && self.use_agent_footer.as_ref(app).has_cli_agent(app)
        {
            let voice_key = AISettings::as_ref(app)
                .voice_input_toggle_key
                .value()
                .to_key_code();
            element = element.with_voice_input_toggle_key(voice_key);
        }

        element = element.with_filtered_blocks(filtered_blocks);

        if let Some(active_filter_editor_block_index) = self.active_filter_editor_block_index {
            element = element.with_active_block_filter_editor(active_filter_editor_block_index);
        }

        if !self.rich_content_views.is_empty() {
            element = element.with_rich_content(
                self.rich_content_views
                    .iter()
                    .map(RichContent::to_block_list_element_render_params),
            );
        }

        if let Some(hovered_block_index) = self.hovered_block_index {
            let block_list = model.block_list();

            // Is this block the first visible item in the viewport? If so, the tool tips should
            // render below their respective buttons or else they'll get cut off by the edge of the
            // element.
            let should_render_tooltip_below_button = self
                .viewport_state(block_list, input_mode, app)
                .iter()
                .next()
                .and_then(|item| item.block_index)
                == Some(hovered_block_index);

            element = element.with_hovered_index(
                hovered_block_index,
                model,
                should_render_tooltip_below_button,
                app,
            );
        }

        if let Some(shared_session) = &self.shared_session {
            let presence_avatars = shared_session.presence_avatars(app);
            let presence_manager = shared_session.presence_manager().clone();
            element = element.with_shared_session_presence(presence_avatars, presence_manager);
        }

        let total_height: Lines = model.block_list().block_heights().summary().height;
        let visible_rows = self.content_element_height_lines(app);

        // Since blocks in a blocklist can have different sizes, we want
        // to make sure we're rendering with enough columns to support them all.
        let agent_view_state = model.block_list().agent_view_state();
        let columns_needed = model
            .block_list()
            .blocks()
            .iter()
            .filter(|b| b.is_visible(agent_view_state))
            .map(|b| b.size().columns)
            .max()
            .unwrap_or(self.size_info.columns);

        let required_terminal_width = self.size_info.cell_width_px.as_f32()
            * (columns_needed as f32)
            + 2. * self.size_info.padding_x_px().as_f32();
        let pane_width = self.content_element_width_px(app);

        let should_be_vertical_scrollable =
            heights_approx_gt(total_height, visible_rows) && is_scrollable;

        // If this is a shared session viewer and the width required to display the entire
        // terminal is larger than the width of the pane, we should make it horizontally scrollable.
        // If there aren't any visible blocks, we should not show a horizontally-scrollable view.
        let should_be_horizontal_scrollable = FeatureFlag::ViewingSharedSessions.is_enabled()
            && model.shared_session_status().is_active_viewer()
            && model
                .block_list()
                .blocks()
                .iter()
                .filter(|b| b.is_visible(agent_view_state))
                .count()
                > 0
            && required_terminal_width > pane_width;

        let block_list = maybe_wrap_terminal_element_in_scrollable(
            should_be_vertical_scrollable,
            should_be_horizontal_scrollable,
            self.blocklist_vertical_scroll_state.clone(),
            self.horizontal_clipped_scroll_state.clone(),
            required_terminal_width,
            theme,
            element,
        );

        let block_list = DropTarget::new(
            block_list,
            TerminalDropTargetData {
                terminal_view: self.view_handle.clone(),
            },
        )
        .finish();

        let is_waterfall_gap_mode =
            matches!(input_mode, InputMode::Waterfall) && model.block_list().active_gap().is_some();
        // In waterfall gap mode, we render the bookmark indicators on the waterfall gap element,
        // not the block list element.
        let element_to_save = if !self.bookmarked_blocks.is_empty() && !is_waterfall_gap_mode {
            self.render_bookmark_indicators(model, block_list, appearance, app)
        } else {
            block_list
        };
        let element =
            SavePosition::new(element_to_save, &self.content_element_position_id).finish();

        let is_waterfall_no_gap_mode =
            matches!(input_mode, InputMode::Waterfall) && model.block_list().active_gap().is_none();

        // If there is an 'inset' to be applied to the blocklist element because the inline menu is
        // visible, we ensure that the blocklist element height constraint accounts for the inline
        // menu, in particular when the total blocklist height is less than the total pane size -
        // in this case, the input would still have room to render underneath the blocklist (since
        // it doesn't take up the whole pane, and would try to render beneath it, rather than shrinking
        // the visible blocklist height and 'sliding' it upwards.
        //
        // On the other hand, when the blocklist height exceeds the pane height and there is no gap,
        // this necessarily means that the input is at the bottom of the viewport, so when the inline
        // menu renders it will necessarily push the blocklist element up because the element is ultimatelyx
        // wrapped in a Shrinkable.
        if let Some(blocklist_inset_due_to_inline_menu) = is_waterfall_no_gap_mode
            .then(|| {
                self.inline_menu_positioner
                    .as_ref(app)
                    .blocklist_top_inset_when_in_waterfall_mode(app)
            })
            .flatten()
        {
            let total_blocklist_height = model
                .block_list()
                .block_heights()
                .summary()
                .height
                .to_pixels(self.size_info.cell_height_px)
                .as_f32();

            let height = self.size_info.pane_height_px.min(
                (total_blocklist_height - blocklist_inset_due_to_inline_menu.as_f32()).max(0.),
            );
            ConstrainedBox::new(element)
                .with_max_height(height)
                .finish()
        } else {
            element
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_waterfall_gap_element(
        &self,
        model: &TerminalModel,
        viewport: &ViewportState,
        active_gap: &Gap,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Stack {
        let input_element = if self.is_input_box_visible(model, app) {
            self.render_input()
        } else {
            // If the active block is running, the input element is empty.
            SavePosition::new(
                ConstrainedBox::new(Empty::new().finish())
                    .with_height(0.)
                    .finish(),
                &self.input.as_ref(app).save_position_id(),
            )
            .finish()
        };
        let waterfall_gap_element = WaterfallGapElement::new(
            self.render_block_list_element(model, InputMode::Waterfall, false, app),
            input_element,
            (model.block_list().block_heights().summary().height)
                .to_pixels(self.size_info.cell_height_px()),
            vec2f(
                self.size_info.pane_width_px().as_f32(),
                active_gap
                    .height()
                    .to_pixels(self.size_info.cell_height_px())
                    .as_f32(),
            ),
            self.size_info.cell_height_px(),
            viewport.scroll_top_in_pixels(),
            self.size_info.pane_height_px(),
            self.inline_menu_positioner.clone(),
        );

        let theme = appearance.theme();

        let scrollable = Scrollable::vertical(
            self.blocklist_vertical_scroll_state.clone(),
            waterfall_gap_element.finish_scrollable(),
            SCROLLBAR_WIDTH,
            theme.disabled_text_color(theme.background()).into(),
            theme.main_text_color(theme.background()).into(),
            Fill::None,
        )
        .finish();
        let gap_element = if !self.bookmarked_blocks.is_empty() {
            self.render_bookmark_indicators(model, scrollable, appearance, app)
        } else {
            scrollable
        };

        Stack::new().with_child(gap_element)
    }

    // In the case of waterfall mode with no gap, we need to handle left and right (for the context menu) clicks
    // in the empty area beneath the input that are typically handled by the block list element.
    pub(super) fn render_waterfall_mode_background(
        &self,
        model: &TerminalModel,
        mut stack: Stack,
        app: &AppContext,
    ) -> Stack {
        let block_list_height_px = {
            (model.block_list().block_heights().summary().height)
                .to_pixels(self.size_info.cell_height_px)
        };
        let input_position_id: Rc<str> = self.input.as_ref(app).save_position_id().into();
        let position_id: Rc<str> = self.waterfall_background_position_id().into();

        /// Retrieves the offset position below the block.
        fn offset_position_outside_block(
            click_position: Vector2F,
            position_id: &str,
            input_position_id: &str,
            block_list_height_px: Pixels,
            ctx: &mut EventContext,
        ) -> Option<Vector2F> {
            let input_height_px = ctx
                .element_position_by_id(input_position_id)
                .map_or(Pixels::zero(), |r| r.height().into_pixels());
            let Some(rect) = ctx.element_position_by_id(position_id) else {
                log::warn!("'{position_id}' position should be saved");
                return None;
            };

            let offset_position = click_position - rect.origin();

            if offset_position.y().into_pixels() > block_list_height_px + input_height_px {
                Some(offset_position)
            } else {
                None
            }
        }

        // Define a click handler that works for both when the blocklist is totally empty and when we are
        // showing the shortcut hints, and for when there is empty space below the input, but there are blocks
        // above it.
        let click_handler = move |child: Box<dyn Element>| -> Box<dyn Element> {
            let saved = position_id.clone();

            SavePosition::new(
                EventHandler::new(child)
                    .on_right_mouse_down(
                        enclose!((position_id, input_position_id) move |ctx, _app, position | {
                                if let Some(position_in_terminal_view) = offset_position_outside_block(
                                    position,
                                    &position_id,
                                    &input_position_id,
                                    block_list_height_px,
                                    ctx,
                                ) {
                                    ctx.dispatch_typed_action(TerminalAction::BlockListContextMenu(
                                        BlockListMenuSource::OutsideBlockRightClick {
                                            position_in_terminal_view,
                                        },
                                    ));
                                    return DispatchEventResult::StopPropagation;
                                }
                                DispatchEventResult::PropagateToParent
                            }
                        ),
                    )
                    .on_left_mouse_down(
                        enclose!((position_id, input_position_id) move |ctx, _app, position| {
                            if offset_position_outside_block(
                                position,
                                &position_id,
                                &input_position_id,
                                block_list_height_px,
                                ctx,
                            )
                            .is_some()
                            {
                                ctx.dispatch_typed_action(TerminalAction::Focus);
                                return DispatchEventResult::StopPropagation;
                            }
                            DispatchEventResult::PropagateToParent
                        }),
                    )
                    .on_middle_mouse_down(
                        enclose!((position_id, input_position_id) move |ctx, _app, position| {
                            if offset_position_outside_block(
                                position,
                                &position_id,
                                &input_position_id,
                                block_list_height_px,
                                ctx,
                            )
                            .is_some()
                            {
                                ctx.dispatch_typed_action(TerminalAction::MiddleClickOnGrid {
                                    position: None,
                                });
                                return DispatchEventResult::StopPropagation;
                            }
                            DispatchEventResult::PropagateToParent
                        }),
                    )
                    .finish(),
                &saved,
            )
            .for_single_frame()
            .finish()
        };

        stack.add_child(
            Flex::column()
                .with_child(Shrinkable::new(1., click_handler(Empty::new().finish())).finish())
                .finish(),
        );
        stack
    }

    pub fn waterfall_background_position_id(&self) -> String {
        format!("waterfall_background__{}", self.view_id)
    }

    /// Renders the bookmark indicators over the given block list or waterfall gap element
    ///
    /// Will only create indicators if there are blocks bookmarked
    fn render_bookmark_indicators(
        &self,
        model: &TerminalModel,
        scrollable_child: Box<dyn Element>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let total_block_height = model.block_list().block_heights().summary().height;
        let mut stack = Stack::new();
        stack.add_child(scrollable_child);

        let mut bookmark_position = IndicatorPositionArg {
            remaining_indicator_count: self.bookmarked_blocks.keys().count(),
            previous_indicator_top: Pixels::zero(),
        };

        let input_mode = *InputModeSettings::as_ref(app).input_mode.value();
        for (index, handle) in self
            .bookmarked_blocks
            .iter()
            .sorted_by(|a, b| match input_mode {
                InputMode::PinnedToBottom | InputMode::Waterfall => Ord::cmp(a.0, b.0),
                InputMode::PinnedToTop => Ord::cmp(b.0, a.0),
            })
        {
            let (offset, indicator) = self.create_bookmark_indicator(
                model,
                handle.clone(),
                *index,
                total_block_height,
                &mut bookmark_position,
                appearance,
                input_mode,
                app,
            );

            stack.add_positioned_child(
                indicator,
                OffsetPositioning::offset_from_parent(
                    offset,
                    ParentOffsetBounds::ParentByPosition,
                    ParentAnchor::TopRight,
                    ChildAnchor::TopRight,
                ),
            );

            let hovered = handle
                .lock()
                .expect("Handle should be available")
                .is_hovered();

            if hovered {
                if let Some(block) = model.block_list().block_at(*index) {
                    let snapshot = render_floating_block_snapshot(block, appearance);

                    stack.add_positioned_child(
                        snapshot,
                        OffsetPositioning::offset_from_parent(
                            vec2f(-BOOKMARK_PREVIEW_OFFSET, offset.y()),
                            ParentOffsetBounds::ParentByPosition,
                            ParentAnchor::TopRight,
                            ChildAnchor::TopRight,
                        ),
                    );
                }
            }
        }

        stack.finish()
    }

    /// Create the indicator for a bookmark
    ///
    /// The indicator will be scaled to match the height of the block relative to the total block
    /// list.
    ///
    /// We also return the offset vector from the top-right of the screen to position the indicator
    /// properly.
    #[allow(clippy::too_many_arguments)]
    fn create_bookmark_indicator(
        &self,
        model: &TerminalModel,
        handle: MouseStateHandle,
        index: BlockIndex,
        total_block_height: Lines,
        bookmark_position: &mut IndicatorPositionArg,
        appearance: &Appearance,
        input_mode: InputMode,
        app: &AppContext,
    ) -> (Vector2F, Box<dyn Element>) {
        let viewport = self.viewport_state(model.block_list(), input_mode, app);
        let start = viewport.top_of_block_in_lines(index);

        let top = bookmark_position.next_indicator_top(
            start,
            total_block_height,
            self.size_info.pane_height_px(),
        );

        let element = Hoverable::new(handle, |state| {
            let base_color = appearance.theme().accent().into_solid();
            let color = if state.is_hovered() {
                base_color
            } else {
                darken(base_color)
            };

            ConstrainedBox::new(
                Container::new(Rect::new().finish())
                    .with_background_color(color)
                    .finish(),
            )
            .with_width(BOOKMARK_INDICATOR_WIDTH)
            .with_height(BOOKMARK_INDICATOR_HEIGHT)
            .finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(TerminalAction::JumpToBookmark(index));
        })
        .finish();

        (vec2f(0., top.as_f32()), element)
    }

    pub fn terminal_position_id(&self) -> String {
        self.position_id.clone()
    }
}
