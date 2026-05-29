use super::*;

impl TerminalView {
    pub(super) fn context_menu_items(
        &self,
        menu_source: &BlockListMenuSource,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<MenuItem<TerminalAction>> {
        let model = self.model.lock();

        let mut items = match (
            menu_source,
            self.highlighted_link.as_ref(),
            self.selected_blocks.is_empty(),
        ) {
            (
                BlockListMenuSource::RegularBlockRightClick { .. }
                | BlockListMenuSource::RichContentBlockRightClick { .. },
                Some(highlighted_link),
                _,
            ) => {
                match highlighted_link {
                    GridHighlightedLink::Url(url) => {
                        let url_content =
                            Some(model.link_at_range(url, RespectObfuscatedSecrets::Yes));
                        url_content
                            .map(|url_content| {
                                vec![MenuItemFields::new("Copy URL")
                                    .with_on_select_action(TerminalAction::ContextMenu(
                                        ContextMenuAction::CopyUrl { url_content },
                                    ))
                                    .into_item()]
                            })
                            .unwrap_or_default()
                    }
                    #[cfg(feature = "local_fs")]
                    GridHighlightedLink::File(file_link) => {
                        let path = file_link.get_inner().absolute_path();
                        let show_in_file_explorer_menu_item_label = if cfg!(target_os = "macos") {
                            "Show in Finder"
                        } else {
                            "Show containing folder"
                        };
                        path.map(|path| {
                            let mut items = vec![
                                MenuItemFields::new("Copy path")
                                    .with_on_select_action(TerminalAction::ContextMenu(
                                        ContextMenuAction::CopyUrl {
                                            url_content: path.to_string_lossy().into(),
                                        },
                                    ))
                                    .into_item(),
                                MenuItemFields::new(show_in_file_explorer_menu_item_label)
                                    .with_on_select_action(TerminalAction::ShowInFileExplorer(
                                        path.clone(),
                                    ))
                                    .into_item(),
                            ];

                            if is_markdown_file(&path) {
                                items.push(
                                    MenuItemFields::new("Open in Warp")
                                        .with_on_select_action(TerminalAction::OpenFileInWarp(path))
                                        .into_item(),
                                );
                                // Because the default for cmd-click is to open in Warp, we also
                                // have an open-in-editor option.
                                items.push(
                                    MenuItemFields::new("Open in editor")
                                        .with_on_select_action(TerminalAction::OpenGridLink(
                                            highlighted_link.clone(),
                                        ))
                                        .into_item(),
                                );
                            }

                            items
                        })
                        .unwrap_or_default()
                    }
                }
            }
            (
                BlockListMenuSource::RegularTextRightClick { .. }
                | BlockListMenuSource::RichContentTextRightClick { .. },
                None,
                true,
            ) => {
                let mut fields = vec![
                    MenuItemFields::new("Copy")
                        .with_on_select_action(TerminalAction::ContextMenu(
                            ContextMenuAction::CopySelectedText,
                        ))
                        .with_key_shortcut_label(keybinding_name_to_display_string(
                            "terminal:copy",
                            ctx,
                        ))
                        .into_item(),
                    MenuItemFields::new("Insert into input")
                        .with_on_select_action(TerminalAction::ContextMenu(
                            ContextMenuAction::InsertSelectedText,
                        ))
                        .into_item(),
                ];
                if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                    fields.extend([
                        MenuItem::Separator,
                        MenuItemFields::new(if FeatureFlag::AgentMode.is_enabled() {
                            *ATTACH_AS_AGENT_MODE_CONTEXT_TEXT
                        } else {
                            ASK_AI_ASSISTANT_TEXT
                        })
                        .with_on_select_action(TerminalAction::ContextMenu(
                            ContextMenuAction::AskAI(if FeatureFlag::AgentMode.is_enabled() {
                                AskAISource::SelectedTerminalText
                            } else {
                                AskAISource::SelectedBlockOrText
                            }),
                        ))
                        .with_key_shortcut_label(Some("⌃ ⇧ Space"))
                        .into_item(),
                    ]);
                }
                fields
            }
            (
                BlockListMenuSource::BlockOverflowButton { .. }
                | BlockListMenuSource::BlockKeybinding { .. }
                | BlockListMenuSource::RegularBlockRightClick { .. }
                | BlockListMenuSource::RichContentBlockRightClick { .. }
                | BlockListMenuSource::OutsideBlockRightClick { .. },
                None,
                false,
            ) => {
                let tail_block_index = self
                    .selected_blocks
                    .tail()
                    .expect("Expected at least one block to be selected.");

                let tail_block = match model.block_list().block_at(tail_block_index) {
                    None => return vec![],
                    Some(block) => block,
                };

                let is_single_selection = self.selected_blocks.is_singleton();
                let is_active_block_selected = self
                    .selected_blocks
                    .is_selected(model.block_list().active_block_index());
                let is_active_block_running = model
                    .block_list()
                    .active_block()
                    .is_active_and_long_running();

                let copy_commands_str = if is_single_selection {
                    "Copy command"
                } else {
                    "Copy commands"
                };
                let copy_str = "Copy";
                let find_str = if is_single_selection {
                    "Find within block"
                } else {
                    "Find within blocks"
                };
                let scroll_to_top_str = if is_single_selection {
                    "Scroll to top of block"
                } else {
                    "Scroll to top of blocks"
                };
                let scroll_to_bottom_str = if is_single_selection {
                    "Scroll to bottom of block"
                } else {
                    "Scroll to bottom of blocks"
                };

                // currently, we don't support share for multi selections
                let is_share_disabled =
                    !is_single_selection || (is_active_block_selected && is_active_block_running);

                let is_ask_ai_disabled = !is_single_selection;

                let is_copy_commands_disabled =
                    is_single_selection && tail_block.command_to_string().trim().is_empty();
                let is_copy_both_disabled =
                    is_copy_commands_disabled && tail_block.output_to_string().trim().is_empty();

                let share_block_label = if FeatureFlag::CreatingSharedSessions.is_enabled()
                    && ContextFlag::CreateSharedSession.is_enabled()
                {
                    "Share block..."
                } else {
                    "Share..."
                };

                let mut items = vec![
                    MenuItemFields::new(copy_str)
                        .with_on_select_action(TerminalAction::ContextMenu(
                            ContextMenuAction::CopyBlocks,
                        ))
                        .with_key_shortcut_label(keybinding_name_to_display_string(
                            "terminal:copy",
                            ctx,
                        ))
                        .with_disabled(is_copy_both_disabled)
                        .into_item(),
                    MenuItemFields::new(copy_commands_str)
                        .with_on_select_action(TerminalAction::ContextMenu(
                            ContextMenuAction::CopyBlockCommands,
                        ))
                        .with_key_shortcut_label(keybinding_name_to_display_string(
                            "terminal:copy_commands",
                            ctx,
                        ))
                        .with_disabled(is_copy_commands_disabled)
                        .into_item(),
                    MenuItemFields::new(share_block_label)
                        .with_on_select_action(TerminalAction::ContextMenu(
                            ContextMenuAction::OpenShareBlockModal {
                                block_index: tail_block_index,
                            },
                        ))
                        .with_key_shortcut_label(keybinding_name_to_display_string(
                            "terminal:open_share_block_modal",
                            ctx,
                        ))
                        .with_disabled(is_share_disabled)
                        .into_item(),
                ];

                if FeatureFlag::CreatingSharedSessions.is_enabled()
                    && ContextFlag::CreateSharedSession.is_enabled()
                {
                    // Sharing a session from a context menu is disabled for multi block selections, restored blocks, and viewers.
                    let is_share_session_disabled = !is_single_selection
                        || model
                            .block_list()
                            .block_at(tail_block_index)
                            .is_none_or(|b| b.is_restored());

                    items.extend(
                        self.session_sharing_context_menu_items(&model, is_share_session_disabled),
                    );
                }

                if WarpDriveSettings::is_warp_drive_enabled(ctx) {
                    items.push(MenuItem::Separator);
                    items.push(
                        MenuItemFields::new("Save as workflow")
                            .with_on_select_action(TerminalAction::ContextMenu(
                                ContextMenuAction::OpenWorkflowModal,
                            ))
                            .with_key_shortcut_label(keybinding_name_to_display_string(
                                "terminal:toggle_teams_modal",
                                ctx,
                            ))
                            .into_item(),
                    );
                }

                if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                    if FeatureFlag::AgentMode.is_enabled() {
                        // We can only attach selected blocks if the input box is visible.
                        if self.is_input_box_visible(&model, ctx) {
                            items.extend([
                                MenuItem::Separator,
                                MenuItemFields::new(*ATTACH_AS_AGENT_MODE_CONTEXT_TEXT)
                                    .with_on_select_action(TerminalAction::ContextMenu(
                                        ContextMenuAction::AskAI(AskAISource::SelectedBlocks),
                                    ))
                                    .with_key_shortcut_label(keybinding_name_to_display_string(
                                        "terminal:ask_ai_assistant",
                                        ctx,
                                    ))
                                    .into_item(),
                            ]);
                        }
                    } else {
                        items.extend([
                            MenuItem::Separator,
                            MenuItemFields::new("Ask Warp AI")
                                .with_on_select_action(TerminalAction::ContextMenu(
                                    ContextMenuAction::AskAI(AskAISource::SelectedBlockOrText),
                                ))
                                .with_key_shortcut_label(keybinding_name_to_display_string(
                                    "terminal:ask_ai_assistant",
                                    ctx,
                                ))
                                .with_disabled(is_ask_ai_disabled)
                                .into_item(),
                        ]);
                    }
                }

                if is_single_selection {
                    let mut copy_output_menu_item = MenuItemFields::new("Copy output")
                        .with_on_select_action(TerminalAction::ContextMenu(
                            ContextMenuAction::CopyBlockOutputs,
                        ))
                        .with_disabled(tail_block.output_grid().is_empty());

                    // If there is an active filter on a block, then we want to display a
                    // Copy filtered output option and assign the "terminal:copy_outputs" keybinding to it.
                    if tail_block.has_active_filter() {
                        items.insert(
                            1,
                            MenuItemFields::new("Copy filtered output")
                                .with_on_select_action(TerminalAction::ContextMenu(
                                    ContextMenuAction::CopyBlockFilteredOutputs,
                                ))
                                .with_key_shortcut_label(keybinding_name_to_display_string(
                                    "terminal:copy_outputs",
                                    ctx,
                                ))
                                .into_item(),
                        );
                        items.insert(2, copy_output_menu_item.into_item());
                    } else {
                        copy_output_menu_item = copy_output_menu_item.with_key_shortcut_label(
                            keybinding_name_to_display_string("terminal:copy_outputs", ctx),
                        );
                        items.insert(2, copy_output_menu_item.into_item());
                    }

                    let mut prompt_items = self.copy_prompt_menu_items(
                        self.input_is_on_git_branch(&model),
                        self.is_rprompt_shown(&model),
                        PromptPosition::Block(tail_block_index),
                    );
                    items.push(MenuItem::Separator);
                    items.append(&mut prompt_items);
                }

                items.append(&mut vec![
                    MenuItem::Separator,
                    MenuItemFields::new(find_str)
                        .with_on_select_action(TerminalAction::ContextMenu(
                            ContextMenuAction::FindWithinBlock,
                        ))
                        .with_key_shortcut_label(keybinding_name_to_display_string(
                            "terminal:find",
                            ctx,
                        ))
                        .into_item(),
                ]);
                items.append(&mut vec![MenuItemFields::new("Toggle block filter")
                    .with_on_select_action(TerminalAction::ToggleBlockFilterOnSelectedOrLastBlock(
                        ToggleBlockFilterSource::ContextMenu,
                    ))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        TOGGLE_BLOCK_FILTER_KEYBINDING,
                        ctx,
                    ))
                    .into_item()]);
                items.append(&mut vec![MenuItemFields::new("Toggle bookmark")
                    .with_on_select_action(TerminalAction::ContextMenu(
                        ContextMenuAction::ToggleBookmark,
                    ))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "terminal:bookmark_selected_block",
                        ctx,
                    ))
                    .into_item()]);

                items.append(&mut vec![
                    MenuItem::Separator,
                    MenuItemFields::new(scroll_to_top_str)
                        .with_on_select_action(TerminalAction::ContextMenu(
                            ContextMenuAction::ScrollToTopOfBlock,
                        ))
                        .with_key_shortcut_label(keybinding_name_to_display_string(
                            "terminal:scroll_to_top_of_selected_block",
                            ctx,
                        ))
                        .into_item(),
                ]);
                items.append(&mut vec![MenuItemFields::new(scroll_to_bottom_str)
                    .with_on_select_action(TerminalAction::ContextMenu(
                        ContextMenuAction::ScrollToBottomOfBlock,
                    ))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "terminal:scroll_to_bottom_of_selected_block",
                        ctx,
                    ))
                    .into_item()]);

                // Add debugging link for command blocks run by the agent
                if is_single_selection {
                    if let Some(metadata) = tail_block.agent_interaction_metadata() {
                        let conversation_id = metadata.conversation_id();

                        // Try to find the exchange ID using the requested command action ID if available,
                        // otherwise use the subagent task ID to get the latest exchange from that task
                        let exchange_id =
                            if let Some(action_id) = metadata.requested_command_action_id() {
                                BlocklistAIHistoryModel::as_ref(ctx)
                                    .conversation(conversation_id)
                                    .and_then(|convo| convo.exchange_id_for_action(action_id))
                            } else if let Some(subagent_task_id) = metadata.subagent_task_id() {
                                BlocklistAIHistoryModel::as_ref(ctx)
                                    .conversation(conversation_id)
                                    .and_then(|convo| convo.get_task(subagent_task_id))
                                    .and_then(|task| task.last_exchange())
                                    .map(|exchange| exchange.id)
                            } else {
                                None
                            };

                        if let Some(exchange_id) = exchange_id {
                            let debugging_items = self.create_copy_debugging_menu_item(
                                exchange_id,
                                *conversation_id,
                                ctx,
                            );
                            if !debugging_items.is_empty() {
                                items.push(MenuItem::Separator);
                                for (button_text, action) in debugging_items {
                                    items.push(
                                        MenuItemFields::new(button_text)
                                            .with_on_select_action(TerminalAction::ContextMenu(
                                                action,
                                            ))
                                            .into_item(),
                                    );
                                }
                            }
                        }
                    }
                }

                items
            }
            (
                BlockListMenuSource::RichContentBlockRightClick { .. }
                | BlockListMenuSource::OutsideBlockRightClick { .. },
                None,
                true,
            ) => {
                // If selection is empty, only show non-block related options
                let mut items = Vec::new();

                if FeatureFlag::CreatingSharedSessions.is_enabled()
                    && ContextFlag::CreateSharedSession.is_enabled()
                {
                    items.extend(self.session_sharing_context_menu_items(&model, false));
                }

                items
            }
            _ => vec![],
        };

        // Add AI block copying actions for AI block right-click, but only when there's no text selection
        // When there's text selection (RichContentTextRightClick), the generic "Copy" menu item for copying selected text is already handled above
        if let BlockListMenuSource::RichContentBlockRightClick {
            rich_content_view_id,
            ..
        } = menu_source
        {
            let hovered_link = self.hovered_rich_content_link_for_view(*rich_content_view_id, ctx);
            for rich_content in self.rich_content_views.iter() {
                if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                    // Find the corresponding AIBlock that has the same entity ID.
                    if ai_metadata.ai_block_handle.id() == *rich_content_view_id {
                        // Add the common copying actions
                        items.extend(self.ai_block_copying_menu_items(
                            *rich_content_view_id,
                            ai_metadata.conversation_id,
                            hovered_link.clone(),
                            &model,
                            ctx,
                        ));

                        // Add fork option for conversation management
                        if !cfg!(target_family = "wasm") {
                            let fork_label = fork_label_for_query(
                                &ai_metadata
                                    .ai_block_handle
                                    .as_ref(ctx)
                                    .get_preceding_user_query(ctx),
                            );
                            items.push(
                                MenuItemFields::new(fork_label)
                                    .with_on_select_action(TerminalAction::ContextMenu(
                                        ContextMenuAction::ForkAIConversationFromBlock {
                                            ai_block_view_id: *rich_content_view_id,
                                            exchange_id: ai_metadata.exchange_id,
                                            conversation_id: ai_metadata.conversation_id,
                                        },
                                    ))
                                    .into_item(),
                            );

                            if ChannelState::channel().is_dogfood() {
                                items.push(
                                    MenuItemFields::new("Fork from here (dev only)")
                                        .with_on_select_action(TerminalAction::ContextMenu(
                                            ContextMenuAction::ForkAIConversationFromExactExchange {
                                                ai_block_view_id: *rich_content_view_id,
                                                exchange_id: ai_metadata.exchange_id,
                                                conversation_id: ai_metadata.conversation_id,
                                            },
                                        ))
                                        .into_item(),
                                );
                            }
                        }

                        // We can't revert restored blocks since we don't restore the full diff
                        if FeatureFlag::RevertToCheckpoints.is_enabled()
                            && !ai_metadata.ai_block_handle.as_ref(ctx).is_restored()
                        {
                            items.push(
                                MenuItemFields::new("Rewind to before here")
                                    .with_on_select_action(TerminalAction::RewindAIConversation {
                                        ai_block_view_id: *rich_content_view_id,
                                        exchange_id: ai_metadata.exchange_id,
                                        conversation_id: ai_metadata.conversation_id,
                                        entrypoint: AgentModeRewindEntrypoint::ContextMenu,
                                    })
                                    .into_item(),
                            );
                        }

                        let debugging_items = self.create_copy_debugging_menu_item(
                            ai_metadata.exchange_id,
                            ai_metadata.conversation_id,
                            ctx,
                        );
                        if !debugging_items.is_empty() {
                            if !items.is_empty() {
                                items.push(MenuItem::Separator);
                            }
                            for (button_text, action) in debugging_items {
                                items.push(
                                    MenuItemFields::new(button_text)
                                        .with_on_select_action(TerminalAction::ContextMenu(action))
                                        .into_item(),
                                );
                            }
                        }
                        break;
                    }
                }
            }
        }

        if matches!(
            menu_source,
            BlockListMenuSource::RegularBlockRightClick { .. }
                | BlockListMenuSource::RegularTextRightClick { .. }
                | BlockListMenuSource::RichContentBlockRightClick { .. }
                | BlockListMenuSource::RichContentTextRightClick { .. }
                | BlockListMenuSource::OutsideBlockRightClick { .. }
        ) {
            // Surface "Clear Blocks" in the right-click menu so it's
            // discoverable without the keyboard shortcut. We skip
            // text-selection contexts (`Regular*TextRightClick` /
            // `RichContentTextRightClick`) because those menus are scoped to
            // actions on the selected text.
            let include_clear = matches!(
                menu_source,
                BlockListMenuSource::RegularBlockRightClick { .. }
                    | BlockListMenuSource::RichContentBlockRightClick { .. }
                    | BlockListMenuSource::OutsideBlockRightClick { .. }
            );
            let clear_menu_item = include_clear
                .then(|| self.clear_buffer_menu_item(&model, ctx))
                .flatten();
            if let Some(clear_menu_item) = clear_menu_item {
                if !items.is_empty() {
                    items.push(MenuItem::Separator);
                }
                items.push(clear_menu_item);
            }

            let current_shell = model.shell_launch_state().available_shell();
            let pane_context_menu_items = self.pane_context_menu_items(current_shell, ctx);
            // Only add the separator if there's something before and after it.
            if !items.is_empty() && !pane_context_menu_items.is_empty() {
                items.push(MenuItem::Separator);
            }
            if !pane_context_menu_items.is_empty() {
                items.extend(pane_context_menu_items);
            }
        }

        items
    }

    /// Builds the "Clear Blocks" entry for the terminal right-click context
    /// menu. Returns `None` when there are no blocks to clear, mirroring the
    /// `TerminalView_NonEmptyBlockList` predicate that gates the
    /// `terminal:clear_blocks` keybinding.
    fn clear_buffer_menu_item(
        &self,
        model: &TerminalModel,
        ctx: &AppContext,
    ) -> Option<MenuItem<TerminalAction>> {
        if model.is_block_list_empty() {
            return None;
        }
        Some(
            MenuItemFields::new("Clear Blocks")
                .with_on_select_action(TerminalAction::ClearBuffer)
                .with_key_shortcut_label(keybinding_name_to_display_string(
                    "terminal:clear_blocks",
                    ctx,
                ))
                .into_item(),
        )
    }

    fn copy_prompt_menu_items(
        &self,
        is_on_git_branch: bool,
        is_rprompt_shown: bool,
        position: PromptPosition,
    ) -> Vec<MenuItem<TerminalAction>> {
        let mut items = vec![MenuItemFields::new("Copy prompt")
            .with_on_select_action(TerminalAction::ContextMenu(ContextMenuAction::CopyPrompt {
                position,
                part: PromptPart::EntirePrompt,
            }))
            .into_item()];

        if is_rprompt_shown {
            items.push(
                MenuItemFields::new("Copy right prompt")
                    .with_on_select_action(TerminalAction::ContextMenu(
                        ContextMenuAction::CopyRprompt,
                    ))
                    .into_item(),
            );
        }

        items.push(
            MenuItemFields::new("Copy working directory")
                .with_on_select_action(TerminalAction::ContextMenu(ContextMenuAction::CopyPrompt {
                    position,
                    part: PromptPart::Pwd,
                }))
                .into_item(),
        );

        if is_on_git_branch {
            items.push(
                MenuItemFields::new("Copy git branch")
                    .with_on_select_action(TerminalAction::ContextMenu(
                        ContextMenuAction::CopyPrompt {
                            position,
                            part: PromptPart::GitBranch,
                        },
                    ))
                    .into_item(),
            )
        }
        items
    }

    fn pane_context_menu_items(
        &self,
        shell: Option<AvailableShell>,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<MenuItem<TerminalAction>> {
        let mut items = vec![];

        if ContextFlag::CreateNewSession.is_enabled() {
            items.extend(vec![
                MenuItemFields::new("Split pane right")
                    .with_on_select_action(TerminalAction::SplitRight(shell.clone()))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:add_right",
                        ctx,
                    ))
                    .into_item(),
                MenuItemFields::new("Split pane left")
                    .with_on_select_action(TerminalAction::SplitLeft(shell.clone()))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:add_left",
                        ctx,
                    ))
                    .into_item(),
                MenuItemFields::new("Split pane down")
                    .with_on_select_action(TerminalAction::SplitDown(shell.clone()))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:add_down",
                        ctx,
                    ))
                    .into_item(),
                MenuItemFields::new("Split pane up")
                    .with_on_select_action(TerminalAction::SplitUp(shell))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:add_up",
                        ctx,
                    ))
                    .into_item(),
            ]);
        }

        let pane_state = self.split_pane_state(ctx);
        if pane_state.is_in_split_pane() {
            let is_maximized = pane_state.is_maximized();
            items.push(
                MenuItemFields::toggle_pane_action(is_maximized)
                    .with_on_select_action(TerminalAction::ToggleMaximizePane)
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "pane_group:toggle_maximize_pane",
                        ctx,
                    ))
                    .into_item(),
            );

            items.push(
                MenuItemFields::new("Close pane")
                    .with_on_select_action(TerminalAction::Close)
                    .with_key_shortcut_label(
                        custom_tag_to_keystroke(CustomAction::CloseCurrentSession.into())
                            .map(|keystroke| keystroke.displayed()),
                    )
                    .into_item(),
            );
        }

        items
    }

    fn input_is_on_git_branch(&self, model: &TerminalModel) -> bool {
        PromptPosition::Input
            .block(model)
            .and_then(Block::git_branch)
            .is_some()
    }

    fn is_rprompt_shown(&self, model: &TerminalModel) -> bool {
        model
            .block_list()
            .active_block()
            .should_display_rprompt(&self.size_info)
    }

    /// Closes all overlays managed by the terminal view and its input. Does not change what
    /// element is focused.
    pub fn close_overlays(&mut self, ctx: &mut ViewContext<Self>) {
        self.close_context_menu(ctx, false);
        self.close_block_filter_editor(ctx);
        self.close_find_bar(ctx);
        self.close_environment_setup_mode_selector(ctx);

        self.input.update(ctx, |input, ctx| {
            input.close_overlays(true, ctx);
        });
    }

    fn close_environment_setup_mode_selector(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_environment_setup_mode_selector_open {
            self.is_environment_setup_mode_selector_open = false;
            ctx.emit(Event::EnvironmentSetupModeSelectorToggled { is_open: false });
            ctx.notify();
        }
    }

    pub(super) fn prompt_context_menu_items(
        &self,
        ctx: &AppContext,
    ) -> Vec<MenuItem<TerminalAction>> {
        let copy_prompt = MenuItemFields::new("Copy prompt")
            .with_on_select_action(TerminalAction::ContextMenu(ContextMenuAction::CopyPrompt {
                position: PromptPosition::Input,
                part: PromptPart::EntirePrompt,
            }))
            .into_item();

        let has_cli_agent_session = CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .is_some();
        let is_agent_view_active = self
            .agent_view_controller
            .as_ref(ctx)
            .agent_view_state()
            .is_active();
        let edit_menu_item = if has_cli_agent_session {
            FeatureFlag::AgentToolbarEditor.is_enabled().then(|| {
                MenuItemFields::new("Edit CLI agent toolbelt")
                    .with_on_select_action(TerminalAction::ContextMenu(
                        ContextMenuAction::EditCLIAgentToolbar,
                    ))
                    .into_item()
            })
        } else if is_agent_view_active {
            FeatureFlag::AgentToolbarEditor.is_enabled().then(|| {
                MenuItemFields::new("Edit agent toolbelt")
                    .with_on_select_action(TerminalAction::ContextMenu(
                        ContextMenuAction::EditAgentToolbar,
                    ))
                    .into_item()
            })
        } else {
            Some(
                MenuItemFields::new("Edit prompt")
                    .with_on_select_action(TerminalAction::ContextMenu(
                        ContextMenuAction::EditPrompt,
                    ))
                    .with_disabled(self.model.lock().shared_session_status().is_active_viewer())
                    .into_item(),
            )
        };

        if *SessionSettings::as_ref(ctx).honor_ps1 {
            let mut items = vec![copy_prompt];
            if self.is_rprompt_shown(&self.model.lock()) {
                items.push(
                    MenuItemFields::new("Copy right prompt")
                        .with_on_select_action(TerminalAction::ContextMenu(
                            ContextMenuAction::CopyRprompt,
                        ))
                        .into_item(),
                );
            }
            if let Some(edit_menu_item) = edit_menu_item {
                items.extend([MenuItem::Separator, edit_menu_item]);
            }
            items
        } else {
            let mut items = vec![copy_prompt];
            let current_prompt_menu_items = self
                .current_prompt
                .as_ref(ctx)
                .copy_menu_items(PromptPosition::Input, ctx);
            if !current_prompt_menu_items.is_empty() {
                items.push(MenuItem::Separator);
                items.extend(current_prompt_menu_items);
            }
            if let Some(edit_menu_item) = edit_menu_item {
                items.extend([MenuItem::Separator, edit_menu_item]);
            }
            items
        }
    }

    pub(super) fn show_prompt_context_menu(
        &mut self,
        position: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        let items = self.prompt_context_menu_items(ctx);
        self.show_context_menu(
            ContextMenuState {
                menu_type: ContextMenuType::Prompt { position },
            },
            items,
            ctx,
        );
    }

    fn input_context_menu_items(
        &self,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<MenuItem<TerminalAction>> {
        let model = self.model.lock();
        let mut items = Vec::new();

        // Input editor is not available for read-only viewers in a shared session,
        // so certain menu items are disabled/removed
        let is_editor_disabled = model.shared_session_status().is_reader();

        // Section 1: Cut, Copy, Copy All, Paste, Share Session
        let (all_current_input_text, selected_input_text) = self.input.read(ctx, |input, ctx| {
            input.editor().read(ctx, |editor, ctx| {
                (editor.buffer_text(ctx), editor.selected_text(ctx))
            })
        });

        if !selected_input_text.is_empty() {
            items.extend([
                MenuItemFields::new("Cut")
                    .with_on_select_action(TerminalAction::InputContextMenuItem(
                        InputContextMenuAction::CutSelectedText,
                    ))
                    .into_item(),
                MenuItemFields::new("Copy")
                    .with_on_select_action(TerminalAction::InputContextMenuItem(
                        InputContextMenuAction::CopySelectedText,
                    ))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "terminal:copy",
                        ctx,
                    ))
                    .into_item(),
            ]);
        }

        if !all_current_input_text.is_empty() & selected_input_text.is_empty() {
            items.push(
                MenuItemFields::new("Select all")
                    .with_on_select_action(TerminalAction::InputContextMenuItem(
                        InputContextMenuAction::SelectAll,
                    ))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "editor_view:select_all",
                        ctx,
                    ))
                    .with_disabled(is_editor_disabled)
                    .into_item(),
            );
        }

        items.push(
            MenuItemFields::new("Paste")
                .with_on_select_action(TerminalAction::InputContextMenuItem(
                    InputContextMenuAction::Paste,
                ))
                .with_key_shortcut_label(keybinding_name_to_display_string("terminal:paste", ctx))
                .with_disabled(is_editor_disabled)
                .into_item(),
        );

        if FeatureFlag::CreatingSharedSessions.is_enabled()
            && ContextFlag::CreateSharedSession.is_enabled()
        {
            items.extend(self.session_sharing_context_menu_items(&model, false));
        }

        // Section 2: AI Command Search, Ask Warp AI
        items.extend([
            MenuItem::Separator,
            MenuItemFields::new("Command search")
                .with_on_select_action(TerminalAction::InputContextMenuItem(
                    InputContextMenuAction::ShowCommandSearch,
                ))
                .with_key_shortcut_label(keybinding_name_to_display_string(
                    "workspace:show_command_search",
                    ctx,
                ))
                .with_disabled(is_editor_disabled)
                .into_item(),
        ]);

        if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
            items.push(
                MenuItemFields::new("AI command search")
                    .with_on_select_action(TerminalAction::InputContextMenuItem(
                        InputContextMenuAction::ShowAICommandSearch,
                    ))
                    .with_key_shortcut_label(keybinding_name_to_display_string(
                        "input:toggle_natural_language_command_search",
                        ctx,
                    ))
                    .with_disabled(is_editor_disabled)
                    .into_item(),
            );

            if !selected_input_text.is_empty() && !FeatureFlag::AgentMode.is_enabled() {
                items.push(
                    MenuItemFields::new("Ask Warp AI")
                        .with_on_select_action(TerminalAction::InputContextMenuItem(
                            InputContextMenuAction::AskWarpAI,
                        ))
                        .into_item(),
                );
            }
        }

        // Section 3: Teams related
        if !all_current_input_text.is_empty() && WarpDriveSettings::is_warp_drive_enabled(ctx) {
            items.extend([
                MenuItem::Separator,
                MenuItemFields::new("Save as workflow")
                    .with_on_select_action(TerminalAction::InputContextMenuItem(
                        InputContextMenuAction::SaveAsWorkflow,
                    ))
                    .into_item(),
            ]);
        }

        // Section 4: input hint text toggle
        if !is_editor_disabled {
            let input_settings = InputSettings::as_ref(ctx);
            let inverse_action = if *input_settings.show_hint_text {
                "Hide"
            } else {
                "Show"
            };
            items.push(MenuItem::Separator);
            items.push(
                MenuItemFields::new(format!("{inverse_action} input hint text"))
                    .with_on_select_action(TerminalAction::InputContextMenuItem(
                        InputContextMenuAction::ToggleInputHintText,
                    ))
                    .into_item(),
            );
        }
        // Section 5: All Pane related
        let current_shell = model.shell_launch_state().available_shell();
        let pane_context_menu_items = self.pane_context_menu_items(current_shell, ctx);
        if !pane_context_menu_items.is_empty() {
            items.push(MenuItem::Separator);
            items.extend(pane_context_menu_items);
        }

        items
    }

    pub(super) fn show_input_context_menu(
        &mut self,
        position: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        let items = self.input_context_menu_items(ctx);

        self.show_context_menu(
            ContextMenuState {
                menu_type: ContextMenuType::Input { position },
            },
            items,
            ctx,
        );

        send_telemetry_from_ctx!(TelemetryEvent::OpenInputContextMenu, ctx);
    }

    pub(super) fn open_workflow_modal(&mut self, ctx: &mut ViewContext<Self>) {
        let selected_block_contents =
            self.selected_block_contents_as_string(BlockEntity::Command, " &&\n", ctx);

        self.open_workflow_modal_with_command(
            selected_block_contents,
            SaveAsWorkflowModalSource::Block,
            ctx,
        );
    }

    pub(super) fn open_block_filter_editor(
        &mut self,
        block_index: BlockIndex,
        opened_from_click: OpenedFromClick,
        ctx: &mut ViewContext<Self>,
    ) {
        self.active_filter_editor_block_index = Some(block_index);
        {
            let model = self.model.lock();
            let active_filter_query = model
                .block_list()
                .block_at(block_index)
                .and_then(|block| block.current_filter())
                .filter(|query| query.is_active)
                .cloned();
            let num_matched_lines = model
                .block_list()
                .num_matched_lines_in_filter_for_block(block_index);
            self.block_filter_editor
                .update(ctx, |block_filter_editor, ctx| {
                    block_filter_editor.open_and_set_filter(
                        active_filter_query,
                        num_matched_lines,
                        ctx,
                    );
                });
        }
        self.focus_block_filter_editor(ctx);
        if matches!(opened_from_click, OpenedFromClick::Yes) {
            send_telemetry_from_ctx!(TelemetryEvent::BlockFilterToolbeltButtonClicked, ctx);
        }
    }

    pub(super) fn close_block_filter_editor(&mut self, ctx: &mut ViewContext<Self>) {
        self.active_filter_editor_block_index = None;
        self.block_filter_editor.update(ctx, |block_filter, ctx| {
            block_filter.reset(ctx);
        });
        ctx.notify();
    }

    pub(super) fn open_workflow_modal_from_block(
        &mut self,
        block_index: BlockIndex,
        ctx: &mut ViewContext<Self>,
    ) {
        // Make the block for which we're showing the modal the only selected block.
        self.reset_selection_to_single_block(block_index, ctx);
        self.scroll_to_if_not_visible(block_index, ctx);

        // Set the command in the modal to the command of the block.
        if let Some(block) = self.model.lock().block_list().block_at(block_index) {
            ctx.emit(Event::OpenWorkflowModalWithCommand(
                block.command_to_string(),
            ))
        }

        send_telemetry_from_ctx!(
            TelemetryEvent::SaveAsWorkflowModal {
                source: SaveAsWorkflowModalSource::Block
            },
            ctx
        );
    }

    pub(super) fn open_workflow_modal_from_ai_generated_workflow(
        &mut self,
        workflow: Workflow,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(Event::OpenWorkflowModalWithTemporary(Box::new(workflow)));

        send_telemetry_from_ctx!(
            TelemetryEvent::SaveAsWorkflowModal {
                source: SaveAsWorkflowModalSource::WarpAIWorkflowCard,
            },
            ctx
        );
    }

    pub fn open_workflow_modal_with_existing(
        &mut self,
        workflow_id: SyncId,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(Event::OpenWorkflowModalWithCloudWorkflow(workflow_id));
        ctx.notify();
    }

    /// Helper method to build alt screen context menu items.
    /// Used both when opening the menu and when rebuilding it (e.g., on pane state changes).
    fn rebuild_alt_screen_context_menu_items(
        &self,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<MenuItem<TerminalAction>> {
        let mut menu_items = Vec::new();
        let model = self.model.lock();

        let semantic_selection = SemanticSelection::as_ref(ctx);
        let selection_string =
            model.selection_to_string(semantic_selection, self.is_inverted_blocklist(ctx), ctx);
        if selection_string.is_some() {
            menu_items.push(
                MenuItemFields::new("Copy")
                    .with_on_select_action(TerminalAction::ContextMenu(
                        ContextMenuAction::CopySelectedText,
                    ))
                    .with_key_shortcut_label(Some("⌘-C"))
                    .into_item(),
            );
            if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                menu_items.extend([
                    MenuItem::Separator,
                    MenuItemFields::new(if FeatureFlag::AgentMode.is_enabled() {
                        *ATTACH_AS_AGENT_MODE_CONTEXT_TEXT
                    } else {
                        ASK_AI_ASSISTANT_TEXT
                    })
                    .with_on_select_action(TerminalAction::ContextMenu(ContextMenuAction::AskAI(
                        AskAISource::SelectedTerminalText,
                    )))
                    .with_key_shortcut_label(Some("⌃-⇧-Space"))
                    .into_item(),
                ]);
            }
        }

        if FeatureFlag::CreatingSharedSessions.is_enabled()
            && ContextFlag::CreateSharedSession.is_enabled()
        {
            menu_items.extend(self.session_sharing_context_menu_items(&model, false));
        }
        let current_shell = model.shell_launch_state().available_shell();
        let mut pane_context_menu_items = self.pane_context_menu_items(current_shell, ctx);
        if !menu_items.is_empty() && !pane_context_menu_items.is_empty() {
            menu_items.push(MenuItem::Separator);
        }
        if !pane_context_menu_items.is_empty() {
            menu_items.append(&mut pane_context_menu_items);
        }
        menu_items
    }

    pub(super) fn alt_screen_context_menu(
        &mut self,
        position: Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        let menu_items = self.rebuild_alt_screen_context_menu_items(ctx);
        self.show_context_menu(
            ContextMenuState {
                menu_type: ContextMenuType::AltScreen { position },
            },
            menu_items,
            ctx,
        );
    }

    pub(super) fn block_list_context_menu(
        &mut self,
        menu_source: &BlockListMenuSource,
        ctx: &mut ViewContext<Self>,
    ) {
        match menu_source {
            BlockListMenuSource::BlockOverflowButton { block_index }
            | BlockListMenuSource::RegularBlockRightClick { block_index, .. } => {
                if !self.selected_blocks.is_selected(*block_index) {
                    // If the context menu is already open, we just want to close
                    // the context menu for the existing selections instead of changing
                    // the selections
                    // TODO(INT-922): It doesn't look like this code is actually being reached. Is this behavior intended?
                    if self.is_context_menu_open() {
                        self.close_context_menu(ctx, true);
                        return;
                    }
                    self.reset_selection_to_single_block(*block_index, ctx);
                }
            }

            BlockListMenuSource::BlockKeybinding { .. } => {
                // If the context menu is already open, we just want to close
                // the context menu for the existing selections instead of changing
                // the selections
                if self.is_context_menu_open() {
                    self.close_context_menu(ctx, true);
                    return;
                }
            }

            BlockListMenuSource::RichContentBlockRightClick { .. }
            | BlockListMenuSource::OutsideBlockRightClick { .. } => {
                // Existing text selections should be deselected when opening a context menu
                // elsewhere. This is already done automatically for RegularBlockRightClick,
                // since block selections clear selected text.
                self.clear_selected_text(ctx);
            }

            BlockListMenuSource::RegularTextRightClick { .. }
            | BlockListMenuSource::RichContentTextRightClick { .. } => {}
        }

        let items = self.context_menu_items(menu_source, ctx);
        if !items.is_empty() {
            self.show_context_menu(
                ContextMenuState {
                    menu_type: ContextMenuType::BlockList {
                        menu_source: *menu_source,
                    },
                },
                items,
                ctx,
            );
        }
    }

    /// Show the context menu that lists the context blocks or selected text attached to an AI query.
    /// The query is the query in the exchange with the given [`AIAgentExchangeId`].
    pub(super) fn open_ai_block_attached_context_menu(
        &mut self,
        ai_block_view_id: EntityId,
        ai_exchange_id: AIAgentExchangeId,
        ai_conversation_id: AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(contexts) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(&ai_conversation_id)
            .map(|conversation| conversation.context_for_exchange(ai_exchange_id))
        else {
            debug_assert!(
                false,
                "Attempted to open attachments menu for AI block with unknown conversation."
            );
            return;
        };

        let font_family = Appearance::as_ref(ctx).monospace_font_family();
        let font_size = Appearance::as_ref(ctx).monospace_font_size();

        const MAX_TEXT_DISPLAY_LENGTH: usize = 23;
        let truncate_text = |text: &String| truncate_from_end(text, MAX_TEXT_DISPLAY_LENGTH);

        let menu_items = contexts
            .filter_map(|context| {
                if let AIAgentContext::Block(block_context) = context {
                    let BlockContext {
                        index: block_index,
                        command,
                        ..
                    } = block_context.as_ref();
                    Some(
                        MenuItemFields::new(truncate_text(command))
                            .with_on_select_action(TerminalAction::SelectAIAttachedBlock(
                                *block_index,
                            ))
                            .with_icon(icons::Icon::Paperclip)
                            .with_font_override(font_family)
                            .with_font_size_override(font_size)
                            .into_item(),
                    )
                } else if let AIAgentContext::SelectedText(selected_text) = context {
                    Some(
                        MenuItemFields::new(truncate_text(selected_text))
                            .with_icon(icons::Icon::Paperclip)
                            .with_font_override(font_family)
                            .with_font_size_override(font_size)
                            .with_no_interaction_on_hover()
                            .into_item(),
                    )
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        self.show_context_menu(
            ContextMenuState {
                menu_type: ContextMenuType::AIBlockAttachedContext { ai_block_view_id },
            },
            menu_items,
            ctx,
        );
    }
}
