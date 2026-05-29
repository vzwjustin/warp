use super::*;

impl TerminalView {
    pub(super) fn alt_mouse_action(
        &mut self,
        mouse_state: &MouseState,
        ctx: &mut ViewContext<Self>,
    ) {
        let escape_sequences = mouse_state
            .to_escape_sequence(self.model.lock().deref())
            .unwrap();
        self.write_user_bytes_to_pty(escape_sequences, ctx);
    }

    pub(super) fn alt_select(&mut self, arg: &SelectAction<Point>, ctx: &mut ViewContext<Self>) {
        match arg {
            SelectAction::Begin {
                point,
                side,
                selection_type,
                ..
            } => {
                self.begin_alt_selection(*point, *side, *selection_type, ctx);
            }
            SelectAction::Update {
                point, side, delta, ..
            } => self.update_alt_selection(*point, *side, delta, ctx),
            SelectAction::End => {
                self.end_alt_selection(ctx);
            }
        }
    }

    pub(super) fn begin_alt_selection(
        &mut self,
        point: Point,
        side: Side,
        selection_type: SelectionType,
        ctx: &mut ViewContext<Self>,
    ) {
        // Clear any active text selections in CLI subagent views, since a new selection
        // is starting on the alt screen (which can be visible simultaneously).
        for subagent_view in self.cli_subagent_views.values() {
            subagent_view.update(ctx, |view, ctx| view.clear_all_selections(ctx));
        }
        self.model.lock().alt_screen_mut().clear_selection();
        self.model
            .lock()
            .alt_screen_mut()
            .start_selection(point, selection_type, side);
        self.is_selecting = true;

        ctx.notify();
    }

    pub(super) fn update_alt_selection(
        &mut self,
        point: Point,
        side: Side,
        _delta: &Lines,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model
            .lock()
            .alt_screen_mut()
            .update_selection(point, side);
        ctx.notify();
    }

    pub(super) fn end_alt_selection(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_selecting {
            self.is_selecting = false;
            self.maybe_copy_selection_to_clipboard(ctx);
            ctx.notify();
        } else {
            log::error!("end_selection dispatched with no pending selection");
        }
    }

    pub(super) fn end_text_selection(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_selecting {
            self.is_selecting = false;
            self.block_text_selection_start_position = None;

            let selected_text = {
                let semantic_selection = SemanticSelection::as_ref(ctx);
                self.model
                    .lock()
                    .selection_to_string(semantic_selection, false, ctx)
                    // It doesn't make sense to allow empty text as AI context.
                    .filter(|text| !text.is_empty())
            };

            // The text selection changed, so clear any previously attached context text.
            self.ai_context_model.update(ctx, |context_model, ctx| {
                context_model.set_pending_context_selected_text(None, false, ctx);
            });

            // A text selection might be a byproduct of a block selection.
            // If there's no renderable text selection, we should clear the text selection.
            if selected_text.is_none() {
                self.clear_selected_text(ctx);
            } else {
                self.maybe_copy_selection_to_clipboard(ctx);
                // Text and block selections are mutually exclusive context sources.
                // When the user makes a non-empty text selection, clear any block selections.
                self.clear_selected_blocks(ctx);
            }

            ctx.notify();
        } else {
            log::error!("end_selection dispatched with no pending selection");
        }
    }

    /// Updates the [`BlocklistAIContextModel`]'s pending context to match currently selected blocks.
    /// Be careful about calling `set_pending_context_block_ids` outside of this function, as invoking
    /// `set_pending_context_block_ids` in multiple places will increase the likelihood of desync.
    fn sync_pending_context_block_ids(&mut self, ctx: &mut ViewContext<Self>) {
        let selected_block_ids = {
            let model = self.model.lock();
            self.selected_blocks
                .to_block_ids(model.block_list())
                .cloned()
                .collect_vec()
        };

        self.ai_context_model.update(ctx, |context_model, ctx| {
            context_model.set_pending_context_block_ids(selected_block_ids, false, ctx);
        })
    }

    /// Sets the pending query follow-up state for this terminal view's AI context model.
    pub fn set_pending_query_state(
        &mut self,
        state: PendingQueryState,
        ctx: &mut ViewContext<Self>,
    ) {
        self.ai_context_model
            .update(ctx, |context_model, ctx| match state {
                PendingQueryState::New { .. } => {
                    context_model.set_pending_query_state_for_new_conversation(
                        AgentViewEntryOrigin::ConversationSelector,
                        ctx,
                    );
                }
                PendingQueryState::Existing { conversation_id } => {
                    context_model.set_pending_query_state_for_existing_conversation(
                        conversation_id,
                        AgentViewEntryOrigin::ConversationSelector,
                        ctx,
                    );
                }
            });
    }

    // Additionally handles side effects of changing block selections (i.e. CMD + F results,
    // Agent Mode context, etc.). The field `self.selected_blocks` should only be mutated as part of
    // a `change_block_selections` or `change_block_selections_to_match_ai_context` invocation.
    pub(super) fn change_block_selections<F>(
        &mut self,
        change_selection: F,
        ctx: &mut ViewContext<Self>,
    ) where
        F: FnOnce(&mut SelectedBlocks),
    {
        change_selection(&mut self.selected_blocks);
        self.update_find_selection(ctx);

        // In AI mode, selected blocks also serve as context. When we change the block
        // selections, we must also update the context
        self.sync_pending_context_block_ids(ctx);
        ctx.emit(Event::SelectedBlocksChanged);
    }

    // Additionally handles side effects of changing block selections (i.e. CMD + F results, etc.),
    // but without re-syncing Agent Mode context. The field `self.selected_blocks` should only be
    // mutated as part of a `change_block_selections` or `change_block_selections_to_match_ai_context`
    // invocation.
    pub(super) fn change_block_selections_to_match_ai_context<F>(
        &mut self,
        change_selection: F,
        ctx: &mut ViewContext<Self>,
    ) where
        F: FnOnce(&mut SelectedBlocks),
    {
        change_selection(&mut self.selected_blocks);
        self.update_find_selection(ctx);

        ctx.emit(Event::SelectedBlocksChanged);
    }

    pub fn integration_test_change_block_selection_to_single(
        &mut self,
        block_index: BlockIndex,
        ctx: &mut ViewContext<Self>,
    ) {
        self.reset_selection_to_single_block(block_index, ctx);
    }

    pub(super) fn block_select(
        &mut self,
        block_action: &BlockSelectAction,
        should_redetermine_focus: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        // If the input suggestions are showing, remove them and don't update block selection.
        // The mouse up event is excluded here as scrollbar and text selection in input suggestion
        // could cause it to misfire (see WAR-274 and WAR-407).
        if self.selected_blocks.is_empty()
            && self
                .input
                .as_ref(ctx)
                .suggestions_mode_model()
                .as_ref(ctx)
                .mode()
                .is_visible()
            && !matches!(block_action, BlockSelectAction::MouseUp { .. })
        {
            self.input
                .update(ctx, |input, ctx| input.close_input_suggestions(true, ctx));
            return;
        }

        // If the context menu is open, clicking somewhere on the block list
        // should only close the context menu, and NOT update block selections.
        if self.is_context_menu_open() {
            self.close_context_menu(ctx, true);
            return;
        }

        match block_action {
            BlockSelectAction::ClearAllBlocks => {
                self.clear_selected_blocks(ctx);
            }
            BlockSelectAction::MouseDown(maybe_block_index) => {
                if let Some(block_index) = maybe_block_index {
                    self.mouse_down_block_index = Some(*block_index);

                    send_telemetry_from_ctx!(
                        TelemetryEvent::BlockSelection(BlockSelectionDetails {
                            cardinality: self.selected_blocks.cardinality(),
                            delta: BlockSelectionDelta::New,
                            is_cmd_down: false,
                            is_shift_down: false,
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
                    // Clear the current block selection upon clicking on a rich content block
                    self.clear_selected_blocks(ctx);

                    // Since rich content blocks cannot be selected, `redetermine_focus` has no way
                    // of knowing whether the user just clicked on a rich content block. To allow
                    // users to attach blocks as context and submit queries quickly, we only divert
                    // the focus away from the input box when we're not in Agent Mode.
                    if !self.ai_input_model.as_ref(ctx).is_ai_input_enabled() {
                        self.focus_terminal(ctx);
                    }
                    // As part of Code Mode V2, we're introducing left and right panels which might be focused
                    // but we want to allow users to click to refocus to a terminal session
                    // so if the terminal isn't focused and a user clicks into the terminal, we want to force focusing the input
                    else if !ctx.is_self_or_child_focused() {
                        self.focus_input_box(ctx);
                    }
                }
            }
            BlockSelectAction::MouseUp {
                block_index,
                is_ctrl_down,
                is_cmd_down,
                is_shift_down,
            } => {
                if let Some(mouse_down_block_index) = self.mouse_down_block_index.take() {
                    // There is a highlighted url and cmd key is held -- don't process this as a block selection.
                    if self.highlighted_link.is_some() && *is_cmd_down {
                        return;
                    }

                    let semantic_selection = SemanticSelection::as_ref(ctx);
                    // Only allow a block to be selected if it's the same block as the mouse down event
                    // and if there's currently no block text selection
                    if mouse_down_block_index == *block_index
                        && self
                            .model
                            .lock()
                            .block_list()
                            .renderable_selection(
                                semantic_selection,
                                self.is_inverted_blocklist(ctx),
                            )
                            .is_none()
                    {
                        let should_toggle_block_selected = if cfg!(target_os = "macos") {
                            *is_cmd_down
                        } else {
                            *is_ctrl_down
                        };

                        if should_toggle_block_selected {
                            // We need to use the next and prev non-hidden indices to
                            // ensure that the tail/pivot of a range selection will never
                            // be a hidden index.
                            let next = self
                                .model
                                .lock()
                                .block_list()
                                .next_non_hidden_block_from_index(*block_index);
                            let prior = self
                                .model
                                .lock()
                                .block_list()
                                .prev_non_hidden_block_from_index(*block_index);

                            // This block's selection needs to be toggled.
                            // If it was already selected, then it will be unselected.
                            // If it wasn't already selected, it will be a new, disjoint selection.
                            self.change_block_selections(
                                |selected_blocks| {
                                    selected_blocks.toggle(*block_index, next, prior);
                                },
                                ctx,
                            );
                        } else if *is_shift_down && !self.selected_blocks.is_empty() {
                            self.change_block_selections(
                                |selected_blocks| {
                                    selected_blocks.range_select(*block_index);
                                },
                                ctx,
                            );
                        } else {
                            self.reset_selection_to_single_block(*block_index, ctx);
                        }

                        if !self.ai_input_model.as_ref(ctx).is_ai_input_enabled() {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::BlockSelection(BlockSelectionDetails {
                                    cardinality: self.selected_blocks.cardinality(),
                                    delta: BlockSelectionDelta::New,
                                    is_cmd_down: *is_cmd_down,
                                    is_shift_down: *is_shift_down
                                }),
                                ctx
                            );
                        } else if !self.selected_blocks.is_empty() {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::AgentModeAttachedBlockContext {
                                    method: AgentModeAttachContextMethod::Mouse
                                },
                                ctx
                            );
                        }
                        self.tips_completed.update(ctx, |tips, ctx| {
                            mark_feature_used_and_write_to_user_defaults(
                                Tip::Hint(TipHint::BlockSelect),
                                tips,
                                ctx,
                            );
                            ctx.notify();
                        });
                    }
                }
            }
        }

        if should_redetermine_focus {
            self.redetermine_global_focus(ctx);
        }
    }

    pub(super) fn block_text_select(
        &mut self,
        arg: &BlockTextSelectAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match arg {
            BlockTextSelectAction::Begin {
                point,
                side,
                selection_type,
                position,
            } => self.begin_block_text_selection(*point, *side, *selection_type, *position, ctx),
            BlockTextSelectAction::Update {
                point,
                side,
                delta,
                position,
            } => self.update_block_text_selection(*point, *side, *delta, *position, ctx),
            BlockTextSelectAction::End => {
                self.end_text_selection(ctx);
            }
        }
    }

    pub(super) fn maybe_copy_selection_to_clipboard(&mut self, ctx: &mut ViewContext<Self>) {
        let selection_settings = SelectionSettings::handle(ctx);
        let semantic_selection = SemanticSelection::as_ref(ctx);
        let model = self.model.lock();
        let selected_text =
            model.selection_to_string(semantic_selection, self.is_inverted_blocklist(ctx), ctx);
        if let Some(selected) = selected_text {
            selection_settings.update(ctx, |selection_settings, ctx| {
                selection_settings
                    .maybe_copy_on_select(ClipboardContent::plain_text(selected), ctx);
            });
        }
    }

    pub(super) fn terminal_is_selecting(
        &self,
        model: &TerminalModel,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let semantic_selection = SemanticSelection::as_ref(ctx);
        (!model.is_alt_screen_active()
            && model
                .block_list()
                .renderable_selection(semantic_selection, self.is_inverted_blocklist(ctx))
                .is_some())
            || model
                .alt_screen()
                .selection_range(semantic_selection)
                .is_some()
    }

    /// Determines if a position in the terminal grid is within an Agent Mode conversation.
    fn is_position_in_agent_mode_conversation(&self, position: &WithinModel<Point>) -> bool {
        // First check if there's an active conversation at all
        let ai_render_context = self.ai_render_context.borrow();
        if !ai_render_context.has_active_conversation() {
            return false;
        }

        // If we're in the alt screen, the content wouldn't be sent to the AI
        if matches!(position, WithinModel::AltScreen(_)) {
            return false;
        }

        // If we're in a block, check if that specific block is part of the active conversation
        if let WithinModel::BlockList(within_block) = position {
            let model = self.model.lock();
            if let Some(block) = model.block_list().block_at(within_block.block_index) {
                // Check if this block has the same visual indicator (pink bar) that shows
                // it's part of the active conversation
                ai_render_context
                    .context_inclusion_state_for_block(block)
                    .is_some()
            } else {
                false
            }
        } else {
            false
        }
    }

    pub(super) fn click_on_grid(
        &mut self,
        position: &WithinModel<Point>,
        modifiers: &ModifiersState,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.terminal_is_selecting(&self.model.lock(), ctx) {
            return;
        }

        let handle = {
            let model = self.model.lock();
            model.secret_at_point(position).map(|(handle, _)| handle)
        };
        let is_in_agent_mode_block = self.is_position_in_agent_mode_conversation(position);
        if let Some(handle) = handle {
            self.open_secret_tool_tip = Some(SecretTooltip::Grid {
                is_agent_mode: is_in_agent_mode_block,
                tooltip: position.replace_inner(handle),
            });
            self.focus_terminal(ctx);
        }

        let should_directly_open_link = should_directly_open_link(modifiers);
        if *GeneralSettings::as_ref(ctx).link_tooltip
            && !should_directly_open_link
            && self.highlighted_link.is_some()
        {
            self.open_grid_link_tool_tip = self.highlighted_link.clone_inner();
            self.focus_terminal(ctx);
        } else {
            self.open_grid_link_tool_tip = None;
        }

        if should_directly_open_link {
            self.maybe_open_link(LinkOpenMethod::CmdClick, position, ctx);
        }
    }

    fn maybe_open_link(
        &mut self,
        link_open_method: LinkOpenMethod,
        position: &WithinModel<Point>,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(link) = self.highlighted_link.as_ref() else {
            return;
        };
        send_telemetry_from_ctx!(
            TelemetryEvent::OpenLink {
                link: link.clone(),
                open_with: link_open_method
            },
            ctx
        );

        match link {
            #[cfg(feature = "local_fs")]
            GridHighlightedLink::File(link) if link.contains(position) => {
                let link = link.get_inner();
                if let Some(path) = link.absolute_path() {
                    self.open_file_path(path, link.line_and_column_num, ctx);
                }
            }
            GridHighlightedLink::Url(url) if url.contains(position) => {
                let model = self.model.lock();
                ctx.notify();
                ctx.open_url(&model.link_at_range(url, RespectObfuscatedSecrets::No));
            }
            _ => (),
        }

        if self.highlighted_link.take(&mut self.model.lock()).is_some() {
            ctx.reset_cursor();
            ctx.notify();
        }
    }

    pub(super) fn middle_click_on_grid(
        &mut self,
        position: &Option<WithinModel<Point>>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.highlighted_link.is_some() {
            // Middle click should open a highlighted link if there is one.
            if let Some(position) = position {
                self.maybe_open_link(LinkOpenMethod::MiddleClick, position, ctx);
            }
        } else {
            // Otherwise, assume that the user wants to middle-click paste.
            self.paste(true, ctx);
        }
    }

    pub(super) fn middle_click_on_input(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus_input_and_clear_selections(ctx);
        self.paste(true, ctx);
    }

    /// Tell the pane group to open a file within Warp.
    pub(super) fn open_file_in_warp(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        if let Some(session) = self
            .active_block_session_id()
            .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id))
        {
            ctx.emit(Event::OpenFileInWarp { path, session })
        }
    }

    pub(super) fn open_code_diff(
        &self,
        view: ViewHandle<CodeDiffView>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(Event::OpenCodeDiff { view });
    }

    pub(super) fn toggle_grid_secret(
        &mut self,
        secret_handle: &WithinModel<SecretHandle>,
        show_secret: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if show_secret && self.model.lock().unobfuscate_secret(secret_handle).is_err() {
            log::warn!(
                "Failed to reveal secret with id {}",
                secret_handle.get_inner().id()
            );
        } else if !show_secret && self.model.lock().obfuscate_secret(secret_handle).is_err() {
            log::warn!(
                "Failed to obfuscate secret with id {}",
                secret_handle.get_inner().id()
            );
        }
        self.dismiss_tooltips(ctx);
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleObfuscateSecret {
                interaction: if show_secret {
                    SecretInteraction::RevealSecret
                } else {
                    SecretInteraction::HideSecret
                }
            },
            ctx
        );
        ctx.notify();
    }

    pub(super) fn toggle_rich_content_secret(
        &mut self,
        tooltip_info: RichContentSecretTooltipInfo,
        show_secret: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        for rich_content in self.rich_content_views.iter() {
            if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                if ai_metadata.ai_block_handle.id() == tooltip_info.view_id {
                    ai_metadata.ai_block_handle.update(ctx, |view, _ctx| {
                        view.set_secret_redaction_state(
                            &tooltip_info.location,
                            &tooltip_info.secret_range,
                            !show_secret,
                        );
                    });
                    break;
                }
            }
        }

        self.dismiss_tooltips(ctx);
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleObfuscateSecret {
                interaction: if show_secret {
                    SecretInteraction::RevealSecret
                } else {
                    SecretInteraction::HideSecret
                }
            },
            ctx
        );
        ctx.notify();
    }

    pub(super) fn copy_grid_secret(
        &mut self,
        secret_handle: &WithinModel<SecretHandle>,
        ctx: &mut ViewContext<Self>,
    ) {
        {
            let model = self.model.lock();
            if let Some(secret) = model.secret_from_handle(secret_handle) {
                let secret_in_model = secret_handle.replace_inner(secret);
                let text = model.string_at_range(&secret_in_model, RespectObfuscatedSecrets::No);
                ctx.clipboard().write(ClipboardContent::plain_text(text));
            }
        }
        send_telemetry_from_ctx!(TelemetryEvent::CopySecret, ctx);
        self.dismiss_tooltips(ctx);
        ctx.notify();
    }

    pub(super) fn copy_rich_content_secret(
        &mut self,
        tooltip_info: RichContentSecretTooltipInfo,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.clipboard()
            .write(ClipboardContent::plain_text(tooltip_info.secret));
        send_telemetry_from_ctx!(TelemetryEvent::CopySecret, ctx);
        self.dismiss_tooltips(ctx);
        ctx.notify();
    }

    pub(super) fn maybe_hover_secret(
        &mut self,
        secret_handle: Option<SecretHandle>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.hovered_secret != secret_handle {
            self.hovered_secret = secret_handle;
            if secret_handle.is_some() {
                ctx.set_cursor_shape(Cursor::PointingHand);
            } else {
                ctx.reset_cursor();
            }
            ctx.notify();
        }
    }

    pub(super) fn block_hover(&mut self, arg: &BlockHoverAction, ctx: &mut ViewContext<Self>) {
        if self.context_menu_state.is_none() {
            match arg {
                BlockHoverAction::Begin { block_index, .. } => {
                    if let Some(hovered_index) = self.hovered_block_index {
                        if *block_index != hovered_index {
                            self.hovered_block_index = Some(*block_index);
                            ctx.notify();
                        }
                    } else {
                        self.hovered_block_index = Some(*block_index);
                        ctx.notify();
                    }
                }
                BlockHoverAction::Clear => {
                    if self.hovered_block_index.is_some()
                        // Don't clear if the user has moved the mouse over the jump to bottom of block button.
                        // This button needs special handling because it's rendered on top of the block list,
                        // not as part of it.
                        && !self.is_jump_to_bottom_of_block_element_hovered()
                    {
                        self.hovered_block_index = None;
                        ctx.notify();
                    }
                }
            }
        }
    }

    pub(super) fn block_snackbar_hover(&mut self, _is_hovered: bool, ctx: &mut ViewContext<Self>) {
        ctx.notify()
    }

    pub(super) fn block_near_snackbar_hover(
        &mut self,
        is_hovered: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.hover_near_snackbar_area = is_hovered;
        ctx.notify()
    }

    pub fn toggle_snackbar_in_active_pane(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_snackbar = !self.show_snackbar;

        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleSnackbarInActivePane {
                show_snackbar: self.show_snackbar
            },
            ctx
        );

        ctx.notify()
    }

    pub(super) fn begin_block_text_selection(
        &mut self,
        point: BlockListPoint,
        side: Side,
        selection_type: SelectionType,
        position: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        // Clear any active text selections in CLI subagent views, since a new selection
        // is starting on the underlying block list.
        for subagent_view in self.cli_subagent_views.values() {
            subagent_view.update(ctx, |view, ctx| view.clear_all_selections(ctx));
        }

        self.block_text_selection_start_position = Some(position);

        self.model
            .lock()
            .block_list_mut()
            .start_selection(point, selection_type, side);
        self.is_selecting = true;

        if self.rich_content_views.is_empty() {
            ctx.notify();
            return;
        }

        let is_inverted_blocklist = self.is_inverted_blocklist(ctx);
        let terminal_model = self.model.lock();
        let block_list = terminal_model.block_list();
        let mut block_cursor = block_list
            .block_heights()
            .cursor::<BlockHeight, BlockHeightSummary>();
        block_cursor.seek(&BlockHeight::from(0.), SeekBias::Right);

        let selection_start_total_index = {
            let mut click_cursor = block_list
                .block_heights()
                .cursor::<BlockHeight, BlockHeightSummary>();
            click_cursor.seek(&BlockHeight::from(point.row), SeekBias::Right);
            click_cursor.start().total_count
        };

        // Loop over each item in the block list. If it's an AI block which doesn't include the point
        // where the user clicked, begin a selection at either the maximum (bottom right) or minimum
        // (top left) point in the block. This is needed to support selections across command blocks
        // and AI blocks since SelectableArea can't start selections outside of its bounds on its own.
        if let Some(active_window_id) = ctx.windows().active_window() {
            while let Some(block_height_item) = block_cursor.item() {
                if let BlockHeightItem::RichContent(RichContentItem { view_id, .. }) =
                    block_height_item
                {
                    if let Some(ai_block) = ctx.view_with_id::<AIBlock>(active_window_id, *view_id)
                    {
                        let x_pos = match selection_type {
                            SelectionType::Rect => Some(position.x()),
                            _ => None,
                        };

                        let ai_block_view = ctx.view(&ai_block);
                        let ai_block_total_index = block_cursor.start().total_count;

                        if (ai_block_total_index < selection_start_total_index
                            && !is_inverted_blocklist)
                            || (ai_block_total_index > selection_start_total_index
                                && is_inverted_blocklist)
                        {
                            ai_block_view.start_selection_at_max_point(selection_type, x_pos);
                        } else if (ai_block_total_index > selection_start_total_index
                            && !is_inverted_blocklist)
                            || (ai_block_total_index < selection_start_total_index
                                && is_inverted_blocklist)
                        {
                            ai_block_view.start_selection_at_min_point(selection_type, x_pos);
                        }
                    }
                }

                block_cursor.next();
            }
        };

        ctx.notify();
    }

    fn update_block_text_selection(
        &mut self,
        point: BlockListPoint,
        side: Side,
        delta: Lines,
        position: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        // When selecting blocks, there is too much noise with the mouse_dragged event,
        // causing a block selection to be mis-interpreted as a text selection. Hence,
        // we check if the move is non-trivial before resetting the block selections.
        if let Some(start_position) = self.block_text_selection_start_position {
            let (start_col, start_row) = (start_position.x(), start_position.y());
            let (curr_col, curr_row) = (position.x(), position.y());
            if (start_col - curr_col).abs() <= MIN_DELTA_FOR_TEXT_SELECTION
                && (start_row - curr_row).abs() <= MIN_DELTA_FOR_TEXT_SELECTION
            {
                return;
            } else {
                self.block_text_selection_start_position = None;
            }
        }

        self.scroll(delta, ctx);

        // Clear the selected block index on mouse drag.
        self.clear_selected_blocks(ctx);
        self.model
            .lock()
            .block_list_mut()
            .update_selection(point, side);

        ctx.notify();
    }

    pub fn is_selecting(&self) -> bool {
        self.is_selecting
    }

    /// Ensures that `block_list_mouse_states` has entries for every block index
    /// currently in the block list. Blocks created outside the normal
    /// `BlockCompleted` event path (e.g. restored conversation command blocks)
    /// would otherwise lack mouse states, which prevents the label hover
    /// tooltip, bookmark button, and filter button from rendering.
    pub(super) fn ensure_mouse_states_for_all_blocks(&mut self) {
        let block_count = self.model.lock().block_list().active_block_index() + BlockIndex::from(1);
        for i in 0..block_count.0 {
            let idx = BlockIndex::from(i);
            self.block_list_mouse_states
                .label_mouse_states
                .entry(idx)
                .or_default();
            self.block_list_mouse_states
                .bookmark_mouse_states
                .entry(idx)
                .or_default();
            self.block_list_mouse_states
                .filter_mouse_states
                .entry(idx)
                .or_default();
        }
    }

    /// Performs a variant of the "clear buffer" action that is special for the agent view.
    /// Returns true iff the clear was successful.
    fn try_clear_buffer_in_agent_view(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let at_least_one_visible_block = self
            .model
            .lock()
            .block_list()
            .has_visible_block_height_item_where(|_| true);

        // If there are no visible blocks, then "clear buffer" is a no-op.
        if !at_least_one_visible_block {
            true
        } else {
            // Otherwise, there are some visible blocks and we need to clear stuff.
            let active_block_is_long_running = self
                .model
                .lock()
                .block_list()
                .active_block()
                .is_active_and_long_running();
            let is_agent_monitoring = self
                .model
                .lock()
                .block_list()
                .active_block()
                .is_agent_monitoring();

            // If there isn't an active long running block, then "clear buffer" just starts a new convo.
            if !active_block_is_long_running {
                self.enter_agent_view_for_new_conversation(
                    None,
                    AgentViewEntryOrigin::ClearBuffer,
                    ctx,
                );
                true
            } else if is_agent_monitoring {
                // Otherwise, if the agent is monitoring this long-running block,
                // then clear just that block and leave the rest of the blocklist in tact.
                self.model.lock().clear_screen(ClearMode::ActiveBlock);
                self.find_model.update(ctx, |find_model, ctx| {
                    find_model.clear_matches(ctx);
                });
                self.update_find_selection(ctx);
                true
            } else {
                // Otherwise, if this is a long-running command that is not agent-monitored,
                // just clear the buffer normally.
                false
            }
        }
    }

    pub(super) fn clear_buffer(&mut self, ctx: &mut ViewContext<Self>) {
        let agent_view_state = self.agent_view_controller.as_ref(ctx).agent_view_state();
        let is_fullscreen_agent_view = agent_view_state.is_fullscreen();
        let is_ambient_agent = self.is_ambient_agent_session(ctx);

        // When in the modal agent view, "clear buffer" has special semantics.
        // Try to clear it specially, but if it wasn't successful, then clear normally.
        if is_fullscreen_agent_view && !is_ambient_agent && self.try_clear_buffer_in_agent_view(ctx)
        {
            ctx.notify();
            return;
        }

        // Don't clear the buffer if the agent is monitoring a long running command
        let is_agent_monitoring = self
            .model
            .lock()
            .block_list()
            .active_block()
            .is_agent_monitoring();

        if is_agent_monitoring {
            return;
        }

        self.clear_selected_blocks(ctx);

        self.ai_context_model.update(ctx, |context_model, ctx| {
            context_model.reset_context_to_default(ctx);
        });

        // Focus the appropriate part of the terminal view (possibly a
        // long-running block, possibly the input field) depending on its
        // current state.
        self.redetermine_global_focus(ctx);

        self.model.lock().clear_screen(ClearMode::ResetAndClear);
        self.find_model.update(ctx, |find_model, ctx| {
            find_model.clear_matches(ctx);
        });

        self.block_list_mouse_states.label_mouse_states.clear();
        self.block_list_mouse_states.bookmark_mouse_states.clear();
        self.block_list_mouse_states.filter_mouse_states.clear();
        self.bookmarked_blocks.clear();

        // Clean up the active AI block if there is one. This MUST be done before
        // clearing the rich content views.
        if let Some(ai_block_handle) = self.active_ai_block(ctx) {
            ai_block_handle.update(ctx, |ai_block, ctx| {
                ai_block.cleanup_block(ctx);
            });
        }

        self.rich_content_views.clear();

        self.update_input_prompt_suggestions_banner_state(ctx);

        // Clear screen will remove all blocks except the started block so insert
        // the label mouse state here to make sure this is handled.
        self.block_list_mouse_states
            .label_mouse_states
            .insert(BlockIndex::zero(), Default::default());
        self.block_list_mouse_states
            .bookmark_mouse_states
            .insert(BlockIndex::zero(), Default::default());
        self.block_list_mouse_states
            .filter_mouse_states
            .insert(BlockIndex::zero(), Default::default());

        self.update_find_selection(ctx);

        // don't consider the terminal view to be in an error state if we cmd+k
        // the failing block away
        if matches!(self.current_state.state, TerminalViewState::Errored) {
            self.set_current_state(TerminalViewState::Normal, ctx);
        }

        self.abort_prompt_and_code_suggestions(ctx);
        self.input.update(ctx, |input, ctx| {
            input
                .editor()
                .update(ctx, |editor, ctx| editor.clear_autosuggestion(ctx))
        });
        self.clear_prompt_suggestions(ctx);

        // Note: we set this here since clear_screen at the TerminalModel and BlockList levels is
        // called much more often (on every new session/block it seems), and we only want to track explicit
        // clear screen actions e.g. Cmd-k.
        self.model.lock().blocklist_has_been_cleared = true;
        ctx.emit(Event::BlockListCleared);

        // If we're currently in a subshell, add another flag to indicate that because we just
        // cleared the existing one.
        if let Some(session) = self
            .active_block_session_id()
            .and_then(|id| self.sessions.as_ref(ctx).get(id))
        {
            if let Some(info) = session.subshell_info() {
                self.warpify_state
                    .add_subshell_separator(info, self.model.clone(), ctx);
            }
        }

        // When we clear the blocklist, the user can't see past AI exchanges anymore, so these conversations should no longer
        // appear active for the terminal view anymore.
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |ai_history_model, ctx| {
            ai_history_model.clear_conversations_in_terminal_view(self.view_id, ctx)
        });

        // No more restored blocks, since we just cleared the buffer
        log::info!("Clearing buffer.  resetting any_session_contains_restored_remote_blocks");
        self.any_session_contains_restored_remote_blocks = false;

        // Since we just cleared blocks, we can just look at the state of the active block
        self.any_session_contains_remote_blocks = self.active_block_is_considered_remote(ctx);
        self.update_focused_terminal_info(ctx);

        ctx.notify();

        if self.block_onboarding_active {
            self.reset_onboarding_blocks(ctx);
        }
    }

    pub(super) fn find_within_block(&mut self, ctx: &mut ViewContext<Self>) {
        self.tips_completed.update(ctx, |tips, ctx| {
            mark_feature_used_and_write_to_user_defaults(
                Tip::Hint(TipHint::BlockAction),
                tips,
                ctx,
            );
            ctx.notify();
        });
        self.update_find_selection(ctx);
        self.show_find_bar(ctx);
    }

    pub(super) fn scroll_to_top_of_topmost_selected_block(&mut self, ctx: &mut ViewContext<Self>) {
        let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
        let block_sort_direction = input_mode.block_sort_direction();
        let sorted_ranges = self.selected_blocks.sorted_ranges(block_sort_direction);
        if let Some(block_index) = sorted_ranges
            .first()
            .and_then(|r| r.range(Some(block_sort_direction)).next())
        {
            self.update_scroll_position_locking(
                ScrollPositionUpdate::ScrollToTopOfBlock { block_index },
                ctx,
            );
        }
    }

    pub(super) fn scroll_to_bottom_of_overhanging_block(
        &mut self,
        overhanging_block: &OverhangingBlock,
        ctx: &mut ViewContext<Self>,
    ) {
        send_telemetry_from_ctx!(TelemetryEvent::JumpToBottomofBlockButtonClicked, ctx);
        self.update_scroll_position_locking(
            ScrollPositionUpdate::ScrollToBottomOfBlock {
                block_index: overhanging_block.block_index(),
            },
            ctx,
        );
    }

    pub(super) fn scroll_to_bottom_of_bottommost_selected_block(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) {
        let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
        let block_sort_direction = input_mode.block_sort_direction();
        let sorted_ranges = self.selected_blocks.sorted_ranges(block_sort_direction);
        if let Some(block_index) = sorted_ranges
            .last()
            .and_then(|r| r.range(Some(block_sort_direction)).last())
        {
            self.update_scroll_position_locking(
                ScrollPositionUpdate::ScrollToBottomOfBlock { block_index },
                ctx,
            );
        }
    }

    pub fn full_prompt(&self, app: &AppContext) -> String {
        self.input.as_ref(app).prompt_and_rprompt_text(app).0
    }

    pub fn prompt_elements(&self, app: &AppContext) -> SessionNavigationPromptElements {
        self.input.as_ref(app).create_prompt_elements(app)
    }

    pub fn session_command_context(&self, app: &AppContext) -> CommandContext {
        let model = self.model.lock();
        let block_list = model.block_list();

        let ai_history_model = BlocklistAIHistoryModel::as_ref(app);

        // Check if the active block is a rich content block.
        if let Some(ai_block_handle) = self.active_ai_block(app) {
            let ai_block = ai_block_handle.as_ref(app);
            if let Some(prompt) = ai_history_model
                .conversation(&ai_block.conversation_id())
                .and_then(|conversation| conversation.latest_user_query())
            {
                return CommandContext::RunningAIBlock {
                    prompt: prompt.to_owned(),
                };
            }
        }

        // Check if the last non-hidden block is a rich content block.
        let block_index = block_list.last_non_hidden_block_by_index();
        if let Some((_, content)) =
            block_list.last_non_hidden_rich_content_block_after_block(block_index)
        {
            if let Some(rich_content) = self.rich_content_views.last() {
                if rich_content.view_id() == content.view_id {
                    if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                        let ai_block = ai_metadata.ai_block_handle.as_ref(app);
                        if let Some(prompt) = ai_history_model
                            .conversation(&ai_block.conversation_id())
                            .and_then(|conversation| conversation.latest_user_query())
                        {
                            return CommandContext::LastRunAIBlock {
                                prompt: prompt.to_owned(),
                            };
                        }
                    }
                }
            }
        }

        // Fall back to existing command context logic for terminal blocks
        let active_block = block_list.active_block();
        let last_block = block_list.last_non_hidden_block();

        match (active_block.is_active_and_long_running(), last_block) {
            // There is an active block running, so we should return the running command.
            (true, _) => CommandContext::RunningCommand {
                running_command: active_block.command_to_string(),
            },
            // There is not active block, so we try to retrieve the last non-hidden block and get its command and timestamp.
            (false, Some(last_block)) => {
                let last_run_command = last_block.command_to_string();

                let mins_since_completion = last_block.completed_ts().map(|completed_ts| {
                    let now = chrono::Local::now();
                    let diff = now.signed_duration_since(*completed_ts);
                    diff.num_minutes()
                });
                CommandContext::LastRunCommand {
                    last_run_command,
                    mins_since_completion,
                }
            }
            // There is no active block and no last non-hidden block, so it is an empty session with no CommandContext.
            (false, None) => CommandContext::None,
        }
    }

    pub(super) fn cut_selected_text_from_input(&mut self, ctx: &mut ViewContext<Self>) {
        let selected_input_text = self.input.read(ctx, |input, ctx| {
            input
                .editor()
                .read(ctx, |editor, ctx| editor.selected_text(ctx))
        });

        self.input.update(ctx, |input, ctx| {
            input.editor().update(ctx, |editor, ctx| {
                editor.backspace(ctx);
            })
        });

        if !selected_input_text.is_empty() {
            ctx.clipboard()
                .write(ClipboardContent::plain_text(selected_input_text));
        }
        send_telemetry_from_ctx!(TelemetryEvent::InputCutSelectedText, ctx);
    }

    pub(super) fn copy_selected_text_from_input(&mut self, ctx: &mut ViewContext<Self>) {
        let selected_input_text = self.input.read(ctx, |input, ctx| {
            input
                .editor()
                .read(ctx, |editor, ctx| editor.selected_text(ctx))
        });

        if !selected_input_text.is_empty() {
            ctx.clipboard()
                .write(ClipboardContent::plain_text(selected_input_text));
        }
        send_telemetry_from_ctx!(TelemetryEvent::InputCopySelectedText, ctx);
    }

    pub(super) fn select_all_text_from_input(&mut self, ctx: &mut ViewContext<Self>) {
        self.input.update(ctx, |input, ctx| {
            input.editor().update(ctx, |editor, ctx| {
                editor.handle_action(&EditorAction::SelectAll, ctx)
            })
        });
        send_telemetry_from_ctx!(TelemetryEvent::InputSelectAll, ctx);
    }

    pub(super) fn paste_in_input(&mut self, ctx: &mut ViewContext<Self>) {
        let clipboard_content = ctx.clipboard().read();

        self.input.update(ctx, |input, ctx| {
            input.system_insert(clipboard_content.plain_text.as_str(), ctx);
            ctx.focus_self();
        });
        send_telemetry_from_ctx!(TelemetryEvent::InputPaste, ctx);
    }

    pub(super) fn command_search_from_input(&mut self, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(TelemetryEvent::InputCommandSearch, ctx);
        ctx.emit(Event::ShowCommandSearch(Default::default()))
    }

    pub(super) fn ai_command_search_from_input(&mut self, ctx: &mut ViewContext<Self>) {
        self.input.update(ctx, |input, ctx| {
            input.handle_action(&InputAction::ShowAiCommandSearch, ctx)
        });
        send_telemetry_from_ctx!(TelemetryEvent::InputAICommandSearch, ctx);
    }

    pub(super) fn save_as_workflow_from_input(&mut self, ctx: &mut ViewContext<Self>) {
        let (all_current_input_text, selected_input_text) = self.input.read(ctx, |input, ctx| {
            input.editor().read(ctx, |editor, ctx| {
                (editor.buffer_text(ctx), editor.selected_text(ctx))
            })
        });

        let command = if selected_input_text.is_empty() {
            all_current_input_text
        } else {
            selected_input_text
        };

        self.open_workflow_modal_with_command(command, SaveAsWorkflowModalSource::Input, ctx);
    }

    pub(super) fn toggle_input_hint_text(&mut self, ctx: &mut ViewContext<Self>) {
        let new_val = InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
            report_if_error!(input_settings.show_hint_text.toggle_and_save_value(ctx));
            *input_settings.show_hint_text
        });

        // Send the same telemetry event that we do from the features page to make data analysis easier.
        send_telemetry_from_ctx!(
            // We purposely keep the FeaturesPageAction event, even though we have moved the setting to AI settings.
            TelemetryEvent::FeaturesPageAction {
                action: "ToggleShowInputHintText".to_string(),
                value: new_val.to_string()
            },
            ctx
        );
    }

    pub(super) fn open_workflow_modal_with_command(
        &mut self,
        command: String,
        source: SaveAsWorkflowModalSource,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(Event::OpenWorkflowModalWithCommand(command));

        send_telemetry_from_ctx!(TelemetryEvent::SaveAsWorkflowModal { source }, ctx);
    }

    pub(super) fn copy_prompt(
        &mut self,
        position: &PromptPosition,
        part: &PromptPart,
        ctx: &mut ViewContext<Self>,
    ) {
        let to_copy = match part {
            PromptPart::EntirePrompt => match position {
                PromptPosition::Block(block_index) => {
                    Self::block_prompt(&self.model.lock(), self.sessions.as_ref(ctx), *block_index)
                }
                PromptPosition::Input => self.input.as_ref(ctx).prompt_and_rprompt_text(ctx).0,
            },
            PromptPart::GitBranch => position
                .block(&self.model.lock())
                .and_then(Block::git_branch)
                .cloned()
                .unwrap_or_default(),
            PromptPart::CondaContext => position
                .block(&self.model.lock())
                .and_then(Block::conda_env)
                .cloned()
                .unwrap_or_default(),
            PromptPart::Pwd => position
                .block(&self.model.lock())
                .and_then(Block::pwd)
                .cloned()
                .unwrap_or_default(),
            PromptPart::VirtualEnv => position
                .block(&self.model.lock())
                .and_then(Block::virtual_env_short_name)
                .unwrap_or_default(),
            PromptPart::ContextChip(kind) => match position {
                PromptPosition::Input => self
                    .current_prompt
                    .as_ref(ctx)
                    .latest_chip_value(kind, ctx)
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                PromptPosition::Block(_) => position
                    .block(&self.model.lock())
                    .and_then(Block::prompt_snapshot)
                    .and_then(|snapshot| snapshot.chip_value(kind))
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            },
        };
        ctx.clipboard().write(ClipboardContent::plain_text(to_copy));

        send_telemetry_from_ctx!(
            TelemetryEvent::ContextMenuCopyPrompt { part: part.clone() },
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
        self.close_context_menu(ctx, true);
    }

    pub(super) fn copy_rprompt(&mut self, ctx: &mut ViewContext<Self>) {
        let rprompt_text_option = self.input.as_ref(ctx).prompt_and_rprompt_text(ctx).1;

        if let Some(rprompt_text) = rprompt_text_option {
            ctx.clipboard()
                .write(ClipboardContent::plain_text(rprompt_text));
        }
    }

    pub(super) fn edit_prompt(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(Event::OpenPromptEditor);
    }

    /// Handle AI entrypoints, routing to AI in blocklist when possible and falling back to the AI
    /// Assistant panel.
    pub(super) fn ask_ai(&mut self, ask_source: &AskAISource, ctx: &mut ViewContext<Self>) {
        let semantic_selection = SemanticSelection::as_ref(ctx);
        let selection_string = self.model.lock().selection_to_string(
            semantic_selection,
            self.is_inverted_blocklist(ctx),
            ctx,
        );

        let ask_data = match (ask_source, selection_string) {
            (
                AskAISource::SelectedBlockOrText | AskAISource::SelectedTerminalText,
                Some(selection_string),
            ) => {
                // Explicitly snapshot and attach the selected text as pending context.
                // This decouples context from the live selection so the text persists
                // even if the user changes their selection afterward.
                self.ai_context_model.update(ctx, |context_model, ctx| {
                    context_model.set_pending_context_selected_text(
                        Some(selection_string.clone()),
                        false,
                        ctx,
                    );
                });

                AskAIType::FromTextSelection {
                    text: Arc::new(selection_string),
                    // In the block list terminal view, selected text is attached directly as Agent Mode context.
                    // In this case, we don't want to re-surface the selected text by rendering "Explain the following..."
                    // However, we want to keep this prompt for long-running commands and the alt-screen view.
                    populate_input_box: !self.is_input_box_visible(&self.model.lock(), ctx),
                }
            }
            (
                AskAISource::Block { .. }
                | AskAISource::LastBlock
                | AskAISource::SelectedBlockOrText,
                _,
            ) => {
                let model = self.model.lock();
                let block_index = match ask_source {
                    AskAISource::Block(block_index) => Some(*block_index),
                    // Since we already checked the match arm for SelectedBlockOrText where there is text selection,
                    // we must be in the selected block case now.
                    AskAISource::SelectedBlockOrText => self.selected_blocks.tail(),
                    AskAISource::LastBlock => model.block_list().last_non_hidden_block_by_index(),
                    AskAISource::SelectedInputText
                    | AskAISource::SelectedTerminalText
                    | AskAISource::SelectedBlocks => None,
                };

                let Some(block) = block_index.and_then(|idx| model.block_list().block_at(idx))
                else {
                    return;
                };

                let input = block.command_to_string();
                let output = block.output_to_string();
                AskAIType::FromBlock {
                    input: Arc::new(input),
                    output: Arc::new(output),
                    exit_code: block.exit_code(),
                    block_index: block.index(),
                }
            }
            (AskAISource::SelectedInputText, _) | (AskAISource::SelectedTerminalText, None) => {
                let selected_input_text = self.input.read(ctx, |input, ctx| {
                    input
                        .editor()
                        .read(ctx, |editor, ctx| editor.selected_text(ctx))
                });

                if selected_input_text.is_empty() {
                    return;
                }

                send_telemetry_from_ctx!(TelemetryEvent::InputAskWarpAI, ctx);
                AskAIType::FromTextSelection {
                    text: Arc::new(selected_input_text),
                    populate_input_box: true,
                }
            }
            (AskAISource::SelectedBlocks, _) => AskAIType::FromBlocks {
                block_indices: self.selected_blocks.block_indices().collect::<HashSet<_>>(),
            },
        };

        if FeatureFlag::AgentMode.is_enabled() {
            self.ask_blocklist_ai(&ask_data, ctx);
        } else {
            ctx.emit(Event::AskAIAssistant(ask_data.clone()));
        }

        self.close_context_menu(ctx, false);
    }

    /// Sets the input mode to AI and locks it. If `query` is `Some`, pre-fills the input box with
    /// the given query and focuses the input box.
    pub fn set_ai_input_mode_with_query(
        &mut self,
        query: Option<&str>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.ai_input_model.update(ctx, |ai_input, ctx| {
            ai_input.set_input_config(
                InputConfig {
                    input_type: InputType::AI,
                    is_locked: true,
                },
                query.is_none(),
                Some(InputTypeAutoDetectionSource::AskAi),
                ctx,
            );
        });

        self.input().update(ctx, |input, ctx| {
            if let Some(query) = query {
                input.replace_buffer_content(query, ctx);
            }

            input.focus_input_box(ctx);
        });
    }

    /// If the input box is visible, update the AI controller's state and potentially prefill the
    /// terminal input with an AI query (depending on whether the text selection has already been
    /// attached as context). If the input box is not visible, make a new pane and do the same.
    pub fn ask_blocklist_ai(&mut self, ask_type: &AskAIType, ctx: &mut ViewContext<Self>) {
        let mut context_block_indices = HashSet::new();

        let (initial_query, auto_suggestion) = match ask_type {
            AskAIType::FromTextSelection {
                text,
                populate_input_box,
            } => {
                if *populate_input_box {
                    let query_prefix = "Explain the following:\n";
                    let formatted_selection = { format!("```\n{}\n```", text.trim()) };
                    let combined_query = Some(format!("{query_prefix}{formatted_selection}"));
                    (combined_query, None)
                } else {
                    (None, None)
                }
            }

            AskAIType::FromBlock { block_index, .. } => {
                context_block_indices.insert(*block_index);
                (None, Some(DEFAULT_ASK_AI_AUTOSUGGESTION_TEXT))
            }
            AskAIType::FromBlocks { block_indices } => {
                context_block_indices.extend(block_indices);
                (None, Some(DEFAULT_ASK_AI_AUTOSUGGESTION_TEXT))
            }

            AskAIType::FromAICommandSearch { query } => {
                let query_prefix = "What is the command to: ";
                (Some(format!("{}{}", query_prefix, query.trim())), None)
            }
        };

        // We don't support attaching blocks as context in new panes.
        if context_block_indices.is_empty() && !self.is_input_box_visible(&self.model.lock(), ctx) {
            ctx.emit(Event::Pane(PaneEvent::NewPaneInAIMode { initial_query }));
            return;
        }

        self.ai_input_model.update(ctx, |ai_input, ctx| {
            ai_input.set_input_type(
                InputType::AI,
                Some(InputTypeAutoDetectionSource::AskAi),
                ctx,
            );
        });

        if !context_block_indices.is_empty() {
            self.change_block_selections(
                |selected_blocks| selected_blocks.reset_to_block_indices(context_block_indices),
                ctx,
            );
        }

        let selected_block_ids = self
            .selected_blocks
            .to_block_ids(self.model.lock().block_list())
            .cloned()
            .collect_vec();

        self.input().update(ctx, |input, ctx| {
            if let Some(initial_query) = initial_query {
                input.replace_buffer_content(initial_query.as_str(), ctx);
            }

            // Don't interfere with potential autosuggestions based on text already in the input
            // buffer.
            if input.buffer_text(ctx).is_empty() {
                if let Some(autosuggestion) = auto_suggestion {
                    input.set_autosuggestion(
                        autosuggestion,
                        AutosuggestionType::AgentModeQuery {
                            context_block_ids: selected_block_ids,
                            was_intelligent_autosuggestion: false,
                        },
                        ctx,
                    );
                }
            }

            input.focus_input_box(ctx);
        });
    }
}
