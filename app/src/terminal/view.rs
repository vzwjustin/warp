mod action;
mod agent_view;
pub mod ambient_agent;
mod block_banner;
pub mod block_onboarding;
pub(crate) mod blocklist_filter;
mod bookmarks;
mod context_menu;
pub mod init;
pub mod inline_banner;
pub mod load_ai_conversation;
pub(crate) mod queued_prompts_panel;
#[cfg(test)]
#[path = "view/queued_prompts_tests.rs"]
mod queued_prompts_tests;
use ai::agent::action::InsertReviewComment;
pub use load_ai_conversation::ConversationRestorationInNewPaneType;
// TODO(advait): if we align on prompt suggestions banner in Input, move code out of inline_banner mod.
pub(crate) mod init_environment;
mod init_project;
pub use init_project::{
    InitActionResult, InitProjectModel, InitProjectModelEvent, InitStepBlock, InitStepKind,
    ProjectScopedRulesResult,
};
use onboarding::callout::{FinalState, OnboardingCalloutViewEvent, OnboardingQuery};
use onboarding::{OnboardingCalloutView, OnboardingKeybindings};

use crate::ai::block_context::BlockContext;
#[cfg(feature = "local_fs")]
use crate::ai::skills::SkillOpenOrigin;
use crate::global_resource_handles::GlobalResourceHandlesProvider;
pub(crate) mod docker_sandbox;
mod link_detection;
mod methods_ai_agent;
mod methods_banners;
mod methods_context_action;
mod methods_context_menu;
mod methods_exec_size;
mod methods_input_find;
mod methods_onboarding;
mod methods_pty_input;
mod methods_rendering;
mod methods_selection;
mod methods_session_bootstrap;
mod methods_session_info;
mod methods_ssh;
mod methods_terminal_events;
mod open_in_warp;
mod pane_impl;
mod passive_suggestions;
mod pending_user_query;
#[cfg(not(target_family = "wasm"))]
pub(crate) mod plugin_instructions_block;
pub mod rich_content;
mod shared_session;
mod shell_terminated_banner;
pub mod ssh_file_upload;
pub(crate) mod ssh_remote_server_choice_view;
pub(crate) mod ssh_remote_server_failed_banner;
mod tab_metadata;
#[cfg(any(test, feature = "integration_tests"))]
mod testing;
mod tooltips;
pub mod use_agent_footer;
mod zero_state_block;

use std::any::Any;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::hash::Hash;
use std::ops::{Deref as _, Range};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::FromStr;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use action::RememberForWarpification;
pub use action::{AgentOnboardingVersion, OnboardingIntention, OnboardingVersion, TerminalAction};
use ai::api_keys::{ApiKeyManager, AwsCredentialsState};
use ai::index::full_source_code_embedding::manager::{BuildSource, CodebaseIndexManager};
use async_channel::{Receiver, Sender};
use base64::Engine as _;
use block_banner::{render_warpification_banner, WarpificationMode, WarpifyBannerState};
pub use block_banner::{WithinBlockBanner, BLOCK_BANNER_HEIGHT};
use block_onboarding::onboarding_agentic_suggestions_block::{
    OnboardingAgenticSuggestionsBlock, OnboardingAgenticSuggestionsBlockEvent, OnboardingChipType,
};
use block_onboarding::onboarding_drive_sharing_block::OnboardingDriveSharingBlock;
use bookmarks::render_floating_block_snapshot;
use chrono::{DateTime, Local, NaiveDateTime};
use command_corrections::rules::generic::history::History as CommandCorrectionsHistoryRule;
use command_corrections::rules::{Rule, RuleId as CommandCorrectionsRuleId};
use command_corrections::{correct_command, Command, Correction, HistoryItem, SessionMetadata};
use enclose::enclose;
pub use init::{
    init, CANCEL_COMMAND_KEYBINDING, TOGGLE_AUTOEXECUTE_MODE_KEYBINDING,
    TOGGLE_HIDE_CLI_RESPONSES_KEYBINDING, TOGGLE_QUEUE_NEXT_PROMPT_KEYBINDING,
};
use init::{INPUT_BOX_VISIBLE_KEY, TOGGLE_BLOCK_FILTER_KEYBINDING};
use inline_banner::{
    render_alias_expansion_banner, render_aws_bedrock_login_banner,
    render_aws_cli_not_installed_banner, render_inline_notifications_discovery_banner,
    render_inline_notifications_error_banner, render_inline_shared_session_ended_banner,
    render_inline_shared_session_started_banner, render_inline_ssh_wrapper_banner,
    render_open_in_warp_banner, render_shell_process_terminated_banner, render_vim_mode_banner,
    AliasExpansionBanner, AliasExpansionBannerAction, AnonymousUserAISignUpBannerState,
    AnonymousUserLoginBannerAction, AwsBedrockLoginBannerAction, AwsBedrockLoginBannerState,
    AwsCliNotInstalledBannerAction, AwsCliNotInstalledBannerState, ByoLlmAuthBannerSessionState,
    OpenInWarpBannerState, SSHBannerAction, SSHBannerState, VimModeBannerAction,
};
pub use inline_banner::{NotificationsDiscoveryBannerAction, NotificationsErrorBannerAction};
use instant::Instant;
use itertools::Itertools;
use lazy_static::lazy_static;
use markdown_parser::FormattedTextFragment;
use parking_lot::FairMutex;
use pathfinder_color::ColorU;
use regex::Regex;
#[cfg(not(target_family = "wasm"))]
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::repositories::RepoDetectionSource;
use serde::Serialize;
use serde_json::json;
use session_sharing_protocol::common::{
    AgentAttachment, LongRunningCommandAgentInteractionState, ParticipantId, Role, RoleRequestId,
    RoleRequestResponse, ServerConversationToken as SessionSharingServerConversationToken,
    WindowSize as SessionSharingWindowSize,
};
use session_sharing_protocol::sharer::{RoleUpdateReason, SessionEndedReason};
use settings::{Setting, ToggleableSetting};
use shared_session::cloud_conversation_continuation::CloudConversationContinuationUiState;
use shared_session::{SharedSessionAdapter, Viewer};
use ssh_file_upload::{FileUpload, FileUploadEvent};
use sum_tree::SeekBias;
use use_agent_footer::UseAgentToolbar;
use uuid::Uuid;
use vec1::vec1;
use warp_core::channel::ChannelState;
use warp_core::command::ExitCode;
use warp_core::context_flag::ContextFlag;
use warp_core::r#async::debounce;
use warp_core::semantic_selection::SemanticSelection;
use warp_core::user_preferences::GetUserPreferences as _;
use warp_util::local_or_remote_path::LocalOrRemotePath;
#[cfg(feature = "local_fs")]
use warp_util::path::LineAndColumnArg;
use warp_util::path::ShellFamily;
use warpui::accessibility::{AccessibilityContent, ActionAccessibilityContent, WarpA11yRole};
use warpui::assets::asset_cache::{AssetCache, AssetCacheEvent};
use warpui::clipboard::ClipboardContent;
use warpui::clipboard_utils::get_image_filepaths_from_paths;
use warpui::elements::new_scrollable::{
    AxisConfiguration, ClippedAxisConfiguration, DualAxisConfig, NewScrollableElement,
    ScrollableAppearance, SingleAxisConfig,
};
use warpui::elements::shimmering_text::ShimmeringTextStateHandle;
use warpui::elements::{
    get_rich_content_position_id, Align, Border, ChildAnchor, ChildView, Clipped,
    ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    DispatchEventResult, DropTarget, DropTargetData, Empty, EventHandler, Expanded, Fill, Flex,
    Hoverable, Icon, LiveElement, MouseStateHandle, NewScrollable, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, PositionedElementAnchor, PositionedElementOffsetBounds,
    Radius, Rect, SavePosition, ScrollStateHandle, Scrollable, ScrollableElement, ScrollbarWidth,
    Shrinkable, Stack, Text,
};
use warpui::event::ModifiersState;
use warpui::fonts::{Cache as FontCache, FamilyId, Properties};
use warpui::geometry::vector::{vec2f, Vector2F};
use warpui::image_cache::ImageType;
use warpui::keymap::Keystroke;
use warpui::notification::{NotificationSendError, RequestPermissionsOutcome, UserNotification};
use warpui::platform::{Cursor, OperatingSystem};
use warpui::r#async::executor::Background;
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::text::SelectionType;
use warpui::ui_components::components::UiComponent;
use warpui::units::{IntoLines, IntoPixels, Lines, Pixels};
use warpui::windowing::WindowManager;
use warpui::{
    end_trace_after_next, record_trace_event, windowing, AccessibilityData, AppContext,
    BlurContext, CursorInfo, Element, Entity, EntityId, EventContext, FocusContext, ModelAsRef,
    ModelHandle, SingletonEntity, Tracked, TypedActionView, View, ViewAsRef, ViewContext,
    ViewHandle, WeakModelHandle, WeakViewHandle, WindowId,
};

use self::link_detection::HighlightedLinkOption;
pub use self::link_detection::{GridHighlightedLink, RichContentLink, RichContentLinkTooltipInfo};
use super::available_shells::AvailableShell;
use super::block_list_viewport::FindMatchScrollLocation;
use super::event::SshLoginStatus;
use super::find::FindOptions;
use super::model::ansi::{SystemDetails, WarpificationUnavailableReason};
use super::model::block::{
    BlockSection, BlocklistEnvVarMetadata, LONG_RUNNING_COMMAND_DURATION_MS,
};
use super::model::blocks::RichContentItem;
use super::model::completions::ShellCompletion;
use super::model::rich_content::RichContentType;
use super::model::secrets::RichContentSecretTooltipInfo;
use super::model::selection::ExpandedSelectionRange;
use super::model::session::SessionBootstrappedEvent;
use super::settings::AltScreenPaddingMode;
use super::ssh::error::{SshErrorBlock, SshErrorBlockEvent, SSH_ERROR_BLOCK_VISIBLE_KEY};
use super::ssh::install_tmux::{
    install_root_tmux_script, install_tmux_script, SshInstallTmuxBlock, SshInstallTmuxBlockEvent,
    SshKeyEvent, TmuxInstallMethod,
};
use super::ssh::root_access::RootAccess;
use super::ssh::ssh_detection::evaluate_warpify_ssh_host;
use super::ssh::util::{
    convert_script_to_one_line, parse_interactive_ssh_command, InteractiveSshCommand,
    SshWarpifyCommand,
};
use super::ssh::warpify::{
    begin_warpify_ssh_session_command, warpify_ssh_session_command, SshWarpifyBlock,
    SshWarpifyBlockEvent,
};
use super::ssh::SSH_WARPIFY_TIMEOUT_DURATION;
use super::warpify::success_block::{WarpifySuccessBlock, WarpifySuccessBlockEvent};
use super::warpify::trigger_state::{SshBlockState, WarpifyState};
use super::warpify::WarpificationSource;
use super::{cli_agent, CLIAgent, GridType, HistoryEvent};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::redaction::redact_secrets;
use crate::ai::agent::todos::popup::{AgentTodosPopupEvent, AgentTodosPopupView};
#[cfg(any(test, feature = "integration_tests"))]
use crate::ai::agent::UserQueryMode;
use crate::ai::agent::{
    AIAgentActionId, AIAgentActionType, AIAgentCitation, AIAgentContext, AIAgentExchangeId,
    AIAgentInput, AIAgentOutputStatus, AIAgentPtyWriteMode, AIAgentTextSection,
    AgentReviewCommentBatch, CancellationReason, EntrypointType, FileLocations,
    FinishedAIAgentOutput, PassiveCodeDiffEntry, PassiveSuggestionResultType,
    PassiveSuggestionTrigger, RenderableAIError, ServerOutputId, ShellCommandCompletedTrigger,
    StaticQueryType,
};
#[cfg(feature = "local_fs")]
use crate::ai::agent::{CurrentHead, DiffBase};
use crate::ai::agent_conversations_model::{AgentConversationsModel, AgentConversationsModelEvent};
use crate::ai::ambient_agents::{
    conversation_output_status_from_conversation, AmbientAgentTaskId, AmbientConversationStatus,
};
use crate::ai::blocklist::agent_view::agent_input_footer::toolbar_item::AgentToolbarItemKind;
use crate::ai::blocklist::agent_view::{
    agent_view_bg_fill, fork_from_last_known_good_state_exchange_id,
    get_agent_view_entry_block_position_id, is_in_cloud_context, AgentViewController,
    AgentViewControllerEvent, AgentViewDisplayMode, AgentViewEntryBlockParams,
    AgentViewEntryOrigin, AgentViewHeaderDisabledTheme, AgentViewHeaderTheme,
    AgentViewZeroStateBlock, AgentViewZeroStateEvent, EphemeralMessageModel,
    ExitConfirmationTrigger, InlineAgentViewHeader, OrchestrationPillBar,
    ENTER_OR_EXIT_CONFIRMATION_WINDOW,
};
use crate::ai::blocklist::block::cli::{CLISubagentView, CLISubagentViewEvent};
use crate::ai::blocklist::block::cli_controller::{
    CLISubagentController, CLISubagentEvent, UserTakeOverReason,
};
use crate::ai::blocklist::block::status_bar::BlocklistAIStatusBarEvent;
use crate::ai::blocklist::block::{AIBlockAction, FinishReason};
use crate::ai::blocklist::codebase_index_speedbump_banner::{
    CodebaseIndexSpeedbumpBannerAction, CodebaseIndexSpeedbumpBannerState, VisibilityState,
};
use crate::ai::blocklist::inline_action::code_diff_view::{CodeDiffView, FileDiff};
use crate::ai::blocklist::model::{
    AIBlockModel, AIBlockModelHelper, AIBlockModelImpl, AIBlockOutputStatus,
};
use crate::ai::blocklist::suggested_agent_mode_workflow_modal::SuggestedAgentModeWorkflowAndId;
use crate::ai::blocklist::suggested_rule_modal::SuggestedRuleAndId;
use crate::ai::blocklist::summarization_cancel_dialog::SummarizationCancelDialog;
use crate::ai::blocklist::telemetry_banner::{should_collect_ai_ugc_telemetry, TelemetryBanner};
use crate::ai::blocklist::usage::conversation_usage_view::{
    ConversationUsageInfo, ConversationUsageView, TimingInfo,
};
use crate::ai::blocklist::{
    ai_brand_color, block_context_from_terminal_model,
    get_ai_block_overflow_menu_element_position_id, get_attached_blocks_chip_element_position_id,
    AIBlock, AIBlockEvent, AutofireAction, BlocklistAIActionEvent, BlocklistAIActionModel,
    BlocklistAIContextEvent, BlocklistAIContextModel, BlocklistAIController,
    BlocklistAIControllerEvent, BlocklistAIHistoryEvent, BlocklistAIHistoryModel,
    BlocklistAIInputEvent, BlocklistAIInputModel, ClientIdentifiers, ConversationStatusUpdate,
    InputConfig, InputType, InputTypeAutoDetectionSource, LegacyPassiveSuggestionsEvent,
    LegacyPassiveSuggestionsModel, MaaPassiveSuggestionsEvent, MaaPassiveSuggestionsModel,
    PassiveSuggestionsModels, PendingAttachment, PendingQueryState, QueuedQueryModel,
    RequestFileEditsFormatKind, ShellCommandExecutor, ShellCommandExecutorEvent,
    SlashCommandRequest, StartAgentExecutor, StartAgentExecutorEvent, StartAgentRequest,
    ATTACH_AS_AGENT_MODE_CONTEXT_TEXT, PRE_REWIND_PREFIX,
};
use crate::ai::conversation_details_panel::ConversationDetailsPanelEvent;
use crate::ai::conversation_utils;
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentModel, AIDocumentVersion};
use crate::ai::execution_profiles::profiles::{AIExecutionProfilesModel, ClientProfileId};
use crate::ai::get_relevant_files::controller::GetRelevantFilesController;
use crate::ai::llms::{LLMId, LLMModelHost, LLMPreferences};
use crate::ai::loading::shimmering_warp_loading_text;
#[cfg(feature = "local_fs")]
use crate::ai::persisted_workspace::PersistedWorkspace;
use crate::ai::predict::prompt_suggestions::{
    has_pending_code_or_unit_test_prompt_suggestion,
    is_accept_prompt_suggestion_bound_to_cmd_enter,
    is_accept_prompt_suggestion_bound_to_ctrl_enter,
};
use crate::ai_assistant::{AskAIType, ASK_AI_ASSISTANT_TEXT};
use crate::antivirus::AntivirusInfo;
use crate::appearance::{Appearance, AppearanceEvent};
use crate::auth::auth_manager::AuthManager;
use crate::auth::auth_state::AuthState;
use crate::auth::auth_view_modal::AuthViewVariant;
use crate::auth::{AuthStateProvider, UserUid};
use crate::autoupdate::{self, get_update_state, AutoupdateStage};
use crate::banner::{
    Banner, BannerAction, BannerEvent, BannerState, BannerTextButton, BannerTextContent,
    DismissalType,
};
use crate::cloud_object::model::actions::ObjectActionType;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObject, GenericStringObjectFormat, JsonObjectType};
#[cfg(feature = "local_fs")]
use crate::code::editor_management::CodeSource;
use crate::code_review::comments::{
    convert_insert_review_comments, AttachedReviewComment, PendingImportedReviewComment,
};
#[cfg(feature = "local_fs")]
use crate::code_review::context::{
    convert_file_diffs_to_diffset_hunks, create_attachment_reference_and_key,
    register_diffset_attachment,
};
#[cfg(feature = "local_fs")]
use crate::code_review::diff_state::LocalDiffStateModel;
use crate::code_review::diff_state::{DiffMode, GitDeltaPreference};
#[cfg(feature = "local_fs")]
use crate::code_review::git_status_update::{
    GitRepoStatusModel, GitStatusMetadata, GitStatusUpdateModel,
};
use crate::code_review::telemetry_event::CodeReviewPaneEntrypoint;
#[cfg(feature = "local_fs")]
use crate::code_review::DiffSetScope;
use crate::context_chips::prompt::Prompt;
use crate::context_chips::prompt_type::PromptType;
use crate::context_chips::ContextChipKind;
use crate::drive::settings::WarpDriveSettings;
use crate::drive::sharing::ShareableObject;
use crate::drive::CloudObjectTypeAndId;
use crate::editor::{AutosuggestionType, CrdtOperation, EditorAction};
use crate::env_vars::env_var_collection_block::{
    EnvVarCollectionBlock, EnvVarCollectionBlockEvent,
};
use crate::env_vars::{CloudEnvVarCollection, EnvVar};
use crate::features::FeatureFlag;
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::{
    CodeReviewPanelArg, PaneConfiguration, PaneEvent, PaneGroupAction, PaneHeaderAction,
    SplitPaneState, TerminalViewResources,
};
use crate::persistence::{self, FinishedCommandMetadata};
use crate::projects::ProjectManagementModel;
use crate::remote_server::manager::{
    RemoteServerInitPhase, RemoteServerManager, RemoteServerManagerEvent,
};
use crate::resource_center::{
    mark_feature_used_and_write_to_user_defaults, Tip, TipHint, TipsCompleted,
};
use crate::search::slash_command_menu::static_commands::commands;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ObjectUid, SyncId};
use crate::server::server_api::ServerApi;
use crate::server::telemetry::{
    self, AgentModeAttachContextMethod, AgentModeEntrypoint, AgentModeRewindEntrypoint,
    AnonymousUserSignupEntrypoint, BlockLatencyInfo, BootstrappingInfo,
    CommandCorrectionAcceptedType, CommandCorrectionEvent, InteractionSource, LinkOpenMethod,
    NotificationAgentVariant, NotificationsTurnedOnSource, PaletteSource, PromptSuggestionViewType,
    SaveAsWorkflowModalSource, SecretInteraction, SharingDialogSource, SlowBootstrapInfo,
    TelemetryEvent, ToggleBlockFilterSource, WorkflowTelemetryMetadata,
};
use crate::session_management::{CommandContext, SessionNavigationPromptElements};
use crate::settings::ai::FocusedTerminalInfo;
#[cfg(feature = "local_fs")]
use crate::settings::import::model::ImportedConfigModel;
use crate::settings::import::view::{SettingsImportEvent, SettingsImportView};
use crate::settings::{
    AISettings, AISettingsChangedEvent, AliasExpansionSettings, AppEditorSettings,
    BlockVisibilitySettings, BlockVisibilitySettingsChangedEvent, CodeSettings, DebugSettings,
    DebugSettingsChangedEvent, EmacsBindingsSettings, FontSettings, FontSettingsChangedEvent,
    InputModeSettings, InputModeSettingsChangedEvent, InputSettings, PaneSettings,
    PaneSettingsChangedEvent, PrivacySettings, PrivacySettingsChangedEvent,
    PrivacySettingsSnapshot, SelectionSettings, VimBannerSettings,
};
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::settings_view::mcp_servers_page::MCPServersSettingsPage;
use crate::settings_view::{flags, SettingsSection};
use crate::shell_indicator::ShellIndicatorType;
use crate::terminal::alias::{check_for_alias_async, AliasedCommand};
use crate::terminal::alt_screen::alt_screen_element::AltScreenElement;
use crate::terminal::alt_screen_reporting::{AltScreenReporting, AltScreenReportingChangedEvent};
use crate::terminal::block_filter::{
    filter_button_position_id, BlockFilterEditor, BlockFilterEditorEvent, BlockFilterQuery,
    OpenedFromClick,
};
use crate::terminal::block_list_element::{
    render_hoverable_block_button, BlockListElement, BlockListMenuSource, BlockListMouseStates,
    BlockSelectAction, BlockTextSelectAction, SnackbarHeaderState, ToolbeltButtonTooltip,
};
use crate::terminal::block_list_viewport::{
    AutoscrollBehavior, InputMode, OverhangingBlock, ScrollPosition, ScrollPositionUpdate,
    ScrollState, ViewportState,
};
use crate::terminal::bootstrap::init_subshell_command;
use crate::terminal::cli_agent_sessions::event::{
    parse_event, CLIAgentEvent, CLIAgentEventPayload, CLIAgentEventType,
    CLI_AGENT_NOTIFICATION_SENTINEL,
};
use crate::terminal::cli_agent_sessions::listener::{
    agent_supports_rich_status, is_agent_supported, CLIAgentSessionListener,
};
#[cfg(not(target_family = "wasm"))]
use crate::terminal::cli_agent_sessions::plugin_manager::{plugin_manager_for, PluginModalKind};
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputEntrypoint, CLIAgentInputState, CLIAgentRichInputCloseReason, CLIAgentSession,
    CLIAgentSessionContext, CLIAgentSessionStatus, CLIAgentSessionsModel,
    CLIAgentSessionsModelEvent,
};
use crate::terminal::color::List;
use crate::terminal::command_corrections_denylist::COMMAND_CORRECTIONS_PREFERRED_DENYLIST;
use crate::terminal::event::{
    AfterBlockCompletedEvent, BlockLatencyData, BlockType, RemoteServerSetupState, TerminalMode,
    UserBlockCompleted,
};
use crate::terminal::find::{BlockGridMatch, BlockListMatch, TerminalFindModel};
use crate::terminal::general_settings::GeneralSettings;
use crate::terminal::grid_size_util::grid_cell_dimensions;
use crate::terminal::input::decorations::InputBackgroundJobOptions;
use crate::terminal::input::inline_menu::InlineMenuPositioner;
use crate::terminal::input::{
    CommandExecutionSource, InputAction, InputEmptyStateChangeReason, InputState, MenuPositioning,
    MenuPositioningProvider,
};
use crate::terminal::keys::TerminalKeybindings;
use crate::terminal::ligature_settings::{should_use_ligature_rendering, LigatureSettings};
use crate::terminal::links::should_directly_open_link;
#[cfg(feature = "local_tty")]
use crate::terminal::local_tty::get_shell_starter;
#[cfg(feature = "local_tty")]
use crate::terminal::local_tty::shell::ShellStarter;
#[cfg(all(windows, feature = "local_tty"))]
use crate::terminal::local_tty::windows::get_user_and_system_env_variable;
use crate::terminal::model::ansi::{ClearMode, Handler};
use crate::terminal::model::block::{
    AgentInteractionMetadata, Block, BlockId, BlockMetadata, LONG_RUNNING_BOTTOM_PADDING_LINES,
};
use crate::terminal::model::blockgrid::BlockGrid;
use crate::terminal::model::blocks::{
    BlockFilter, BlockHeight, BlockHeightItem, BlockHeightSummary, BlockList, BlockListPoint, Gap,
    RemovableBlocklistItem,
};
use crate::terminal::model::escape_sequences::{self, EscCodes, ToEscapeSequence, C1};
use crate::terminal::model::grid::grid_handler::{FragmentBoundary, TermMode};
use crate::terminal::model::index::{Point, Side};
use crate::terminal::model::mouse::MouseState;
use crate::terminal::model::selection::{SelectAction, SelectionDirection};
use crate::terminal::model::session::active_session::ActiveSession;
use crate::terminal::model::session::{
    BootstrapSessionType, Session, SessionId, SessionType, Sessions, SessionsEvent,
};
use crate::terminal::model::terminal_model::{
    BlockIndex, BlockSelectionCardinality, SelectedBlocks, TerminalInputState, WithinModel,
};
use crate::terminal::model::{ObfuscateSecrets, RespectObfuscatedSecrets, SecretHandle};
use crate::terminal::model_events::{AnsiHandlerEvent, ModelEvent, ModelEventDispatcher};
use crate::terminal::recorder::PtyRecorder;
use crate::terminal::safe_mode_settings::get_secret_obfuscation_mode;
use crate::terminal::session_settings::{
    NotificationsMode, NotificationsSettings, SessionSettings, SessionSettingsChangedEvent,
    ToolbarChipSelection, DEFAULT_THRESHOLD_FOR_LONG_RUNNING_NOTIFICATION,
};
use crate::terminal::settings::{TerminalSettings, TerminalSettingsChangedEvent};
use crate::terminal::shared_session::role_change_modal::{
    RoleChangeCloseSource, RoleChangeOpenSource,
};
use crate::terminal::shared_session::{
    SharedSessionActionSource, SharedSessionScrollbackType, SharedSessionSource,
    SharedSessionStatus,
};
use crate::terminal::ssh::ssh_detection::SshInteractiveSessionDetected;
use crate::terminal::view::block_onboarding::onboarding_prompt_block::OnboardingPromptBlock;
use crate::terminal::view::init_environment::mode_selector::{
    EnvironmentSetupMode, EnvironmentSetupModeSelector, EnvironmentSetupModeSelectorEvent,
};
use crate::terminal::view::init_environment::{InitEnvironmentBlock, InitEnvironmentBlockEvent};
use crate::terminal::view::inline_banner::{
    render_agent_mode_setup_banner, AgentModeSetupSpeedbumpBannerAction,
    AgentModeSetupSpeedbumpBannerState, AliasExpansionBannerState,
    NotificationsDiscoveryBannerState, NotificationsErrorBannerState, PromptSuggestionBannerState,
    VimModeBannerState,
};
use crate::terminal::view::passive_suggestions::PromptSuggestionResolution;
pub use crate::terminal::view::rich_content::{
    AIBlockMetadata, AgentViewEntryMetadata, RichContent, RichContentInsertionPosition,
    RichContentMetadata,
};
use crate::terminal::view::ssh_file_upload::FileUploadId;
use crate::terminal::view::ssh_remote_server_choice_view::{
    SshRemoteServerChoiceView, SshRemoteServerChoiceViewEvent,
};
use crate::terminal::view::ssh_remote_server_failed_banner::{
    SshRemoteServerFailedBanner, SshRemoteServerFailedBannerEvent,
};
use crate::terminal::view::telemetry::PromptSuggestionFallbackReason;
use crate::terminal::view::zero_state_block::TerminalViewZeroStateBlock;
use crate::terminal::warpify::render::render_subshell_separator;
use crate::terminal::warpify::settings::WarpifySettings;
use crate::terminal::warpify::SubshellSource;
use crate::terminal::waterfall_gap_element::WaterfallGapElement;
use crate::terminal::{
    block_list_element::BlockHoverAction,
    // find::{Event as FindEvent, Find, FindDirection},
    input::{Event as InputEvent, Input, INPUT_A11Y_HELPER, INPUT_A11Y_LABEL},
    model::block::SerializedBlock,
    shell::ShellType,
    terminal_size_element::TerminalSizeElement,
    TerminalModel,
};
use crate::terminal::{
    color, element_size_at_last_frame, height_in_range_approx, heights_approx_eq,
    heights_approx_gt, prompt, AudibleBell, BlockListSettings, BlockListSettingsChangedEvent,
    CellSizeAndWindowPadding, History, HistoryEntry, ShellHost, ShellLaunchData, SizeInfo,
    SizeUpdate, SizeUpdateReason,
};
use crate::themes::theme::WarpTheme;
use crate::throttle::throttle;
use crate::ui_components::icons::{self};
use crate::util::bindings::{
    custom_tag_to_keystroke, keybinding_name_to_display_string, keybinding_name_to_keystroke,
    set_custom_keybinding, CustomAction,
};
use crate::util::clipboard::clipboard_content_with_escaped_paths;
use crate::util::color::darken;
#[cfg(feature = "local_fs")]
use crate::util::file::external_editor::{settings::EditorLayout, EditorSettings};
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::{is_markdown_file, resolve_file_target, FileTarget};
use crate::util::repo_detection::{detect_possible_git_repo, RepoDetectionSessionType};
use crate::util::truncation::truncate_from_end;
use crate::view_components::action_button::{ActionButton, ButtonSize, KeystrokeSource};
use crate::view_components::find::{Event as FindEvent, Find, FindDirection, FindWithinBlockState};
use crate::view_components::{DismissibleToast, ToastFlavor};
use crate::workflows::workflow::Workflow;
use crate::workflows::WorkflowSelectionSource;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspace::view::cloud_agent_capacity_modal::CloudAgentCapacityModalVariant;
use crate::workspace::{
    CommandSearchOptions, ForkAIConversationParams, ForkFromExchange,
    ForkedConversationDestination, OneTimeModalModel, ToastStack, WorkspaceAction,
};
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};
use crate::workspaces::workspace::CustomerType;
use crate::{
    report_if_error, safe_error, safe_warn, send_telemetry_from_ctx, send_telemetry_on_executor,
    send_telemetry_sync_from_ctx, AIAgentActionResultType, AIRequestUsageModel,
    ActiveSession as WindowActiveSession,
};

lazy_static! {
    // A set of commands that perform minimal work that we use as a baseline to measure the latency of blocks.
    // Note that while the empty command doesn't invoke pre-exec, it still does get a newline from
    // the shell, and runs precmd.
    static ref BASELINE_COMMANDS: HashSet<&'static str> = HashSet::from(["", "pwd", "whoami", "cd"]);

    // A regex to detect a class of error strings indicating the ControlMaster connection is
    // broken.
    pub static ref CONTROL_MASTER_ERROR_REGEX: regex::Regex =
        regex::Regex::new(r"(?m)^channel (\d)+: open failed:")
        .expect("The regex should compile");

    /// A regex to detect Unix- or Windows-style line feeds in text.
    pub static ref LINEFEED_REGEX: Regex = Regex::new("\r?\n").expect("should not fail to compile regex");

    /// Show the jump to bottom of block button if more than this height of the block is in view.
    static ref JUMP_TO_BOTTOM_OVERHANG_THRESHOLD_PX: Pixels = (70.).into_pixels();

    static ref JUMP_TO_BOTTOM_OF_BLOCK_ICON_SIZE_PX: Pixels = (20.).into_pixels();
    static ref JUMP_TO_BOTTOM_OF_BLOCK_BUTTON_PADDING_PX: Pixels = (4.).into_pixels();
    static ref JUMP_TO_BOTTOM_OF_BLOCK_CORNER_RADIUS_PX: Pixels = (4.).into_pixels();
    static ref JUMP_TO_BOTTOM_OF_BLOCK_TOOLTIP_OFFSET_Y_PX: Pixels = (-5.).into_pixels();


    static ref SUBSHELL_BANNER_DELAY_DURATION: Duration = if cfg!(feature = "integration_tests") {
        Duration::from_secs(0)
    } else {
        Duration::from_secs(1)
    };

    /// The delay between receiving the RC file snippet for subshell bootstrap and writing the
    /// subshell InitShell command to the PTY.
    ///
    /// This is necessary because some subshells may execute initialization commands (for example,
    /// `poetry shell` executes a command that sources the project's python virtualenv), and we
    /// want to submit the InitShell command _after_ those commands have finished execution.
    ///
    /// This is purely a heuristic and may be subject to change based on user reports.
    static ref TRIGGER_RC_FILE_SUBSHELL_BOOTSTRAP_DELAY: Duration = Duration::from_millis(100);

    static ref DEFAULT_IGNORED_RULES_FOR_COMMAND_CORRECTIONS: [CommandCorrectionsRuleId; 1] = [
        CommandCorrectionsHistoryRule.id()
    ];

    /// A list of alt-screen apps that are known to cause problems when resizing
    /// during initialization.
    ///
    /// See [`TerminalView::resize_alt_screen_redundantly`] for more details.
    static ref ALT_SCREEN_APPS_WITH_RESIZE_PROBLEMS: HashSet<&'static str> = HashSet::from(["emacs"]);

    /// A list of alt-screen apps that should never use custom-padding in the alt-screen
    /// and should instead match blocklist padding.
    ///
    /// See [`TerminalView::resize_alt_screen_redundantly`] for more details.
    static ref ALT_SCREEN_APPS_THAT_MUST_MATCH_BLOCKLIST_PADDING: HashSet<&'static str> = HashSet::from(["k9s", "lazygit"]);
}

pub const AI_CONTROL_PANEL_MARGIN: f32 = 10.;

pub const OVERFLOW_BUTTON_OFFSET_X: f32 = -3.;
pub const MAX_WAKEUPS_PER_SECOND: u64 = 60;
pub const WAKEUP_THROTTLE_PERIOD: Duration =
    Duration::from_micros(1000 * 1000 / MAX_WAKEUPS_PER_SECOND);

pub const EXECUTE_PENDING_COMMAND_DELAY: Duration = Duration::from_millis(100);

pub const WARP_PROMPT_HEIGHT_LINES: f32 = 0.9;

const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;

/// Width of the bookmark indicator
const BOOKMARK_INDICATOR_WIDTH: f32 = 15.;
/// Offset from the right for the bookmark preview
const BOOKMARK_PREVIEW_OFFSET: f32 = 20.;
/// Minimum gap between two bookmark indicators
const BOOKMARK_MIN_GAP: f32 = 4.;
/// Height of a bookmark indicator
const BOOKMARK_INDICATOR_HEIGHT: f32 = 4.;

const BRACKETED_PASTE_PREFIX: &str = "\x1b[200~";
const BRACKETED_PASTE_SUFFIX: &str = "\x1b[201~";

/// Duration before we consider a session to have failed bootstrapping.
const BOOTSTRAP_FAILED_DURATION: Duration = Duration::from_secs(7);
/// Duration before we consider a session invoked from an env vars object to
/// have failed bootstrapping. The longer duration is meant to account for
/// a user needing to type in one or many secret manager passwords
/// during the bootstrap period.
const ENV_VAR_BOOTSTRAP_FAILED_DURATION: Duration = Duration::from_secs(60);
const KNOWN_ISSUES_URL: &str =
    "https://docs.warp.dev/support-and-community/troubleshooting-and-support/known-issues";

/// Link to supported custom prompts.
const PROMPT_COMPATIBILITY_URL: &str =
    "https://docs.warp.dev/terminal/appearance/prompt#custom-prompt-compatibility-table";

/// Link to troubleshooting steps for ControlMaster errors.
const CONTROLMASTER_ISSUES_URL: &str =
    "https://docs.warp.dev/terminal/warpify/ssh-legacy#troubleshooting";

/// Link to instructions on how to update p10k.
const P10K_UPDATE_INSTRUCTIONS_URL: &str =
    "https://github.com/romkatv/powerlevel10k#how-do-i-update-powerlevel10k";

const CONTEXT_MENU_WIDTH: f32 = 280.;

/// The minimum amount of mouse-drag to consider a selection to
/// be a text-selection as opposed to mouse-drag noise.
/// Roughly determined by trial-and-error.
const MIN_DELTA_FOR_TEXT_SELECTION: f32 = 0.5;

/// Notifications-specific info
/// TODO (suraj): add documentation for notifications in gitbook
const NOTIFICATIONS_LEARN_MORE_URL: &str =
    "https://docs.warp.dev/terminal/more-features/notifications";
pub const NOTIFICATIONS_TROUBLESHOOT_URL: &str =
    "https://docs.warp.dev/terminal/more-features/notifications#troubleshooting-notifications";

const DEBOUNCE_PERIOD: Duration = Duration::from_millis(40);

/// Key used in user defaults to save whether the user has seen the banner.
pub const ALIAS_EXPANSION_BANNER_SEEN_KEY: &str = "AliasExpansionBannerSeen";

/// Delay between receiving preexec hook for a command we want to auto-warpify
/// and triggering the warpification (subshell bootstrapping).
/// Reached this number after experimenting with different values to find a reliable delay.
const AUTO_WARPIFY_DELAY: u64 = 1000;

/// Binding names to be customized if the user indicates they prefer
/// Emacs-style keybindings instead of IDE-style keybindings.
/// These are specific to non-MacOS desktop platforms.
const SELECT_ALL_BINDING_NAME: &str = "editor_view:select_all";
const MOVE_LINE_START_BINDING_NAME: &str = "editor_view:move_to_line_start";
const MOVE_LINE_END_BINDING_NAME: &str = "editor_view:move_to_line_end";

const DEFAULT_AI_BLOCK_HEIGHT: f32 = 96.;

pub const DEFAULT_ASK_AI_AUTOSUGGESTION_TEXT: &str = "What happened here?";

const WARP_MD_PATH: &str = "WARP.md";

pub const LONG_RUNNING_AGENT_REQUESTED_COMMAND_CONTEXT_KEY: &str = "LongRunningRequestedCommand";
pub const LONG_RUNNING_AGENT_REQUESTED_COMMAND_USER_TOOK_OVER_CONTEXT_KEY: &str =
    "LongRunningRequestedUserTookOverCommand";

/// We only auto open the code review pane if the pane it's getting opened from has a certain width
const MINIMUM_WIDTH_TO_AUTO_OPEN_PANE: f32 = 600.0;

lazy_static! {
    static ref CTRL_SHIFT_A_KEYSTROKE: Keystroke = Keystroke {
        key: "A".into(),
        ctrl: true,
        shift: true,
        ..Default::default()
    };
    static ref CTRL_A_KEYSTROKE: Keystroke = Keystroke {
        key: "a".into(),
        ctrl: true,
        ..Default::default()
    };
    static ref CTRL_E_KEYSTROKE: Keystroke = Keystroke {
        key: "e".into(),
        ctrl: true,
        ..Default::default()
    };

    /// The padding between the left of the element and where the grid contents (either via the
    /// `BlockList` or the `AltScreen`) should be rendered.
    pub static ref PADDING_LEFT: f32 = 16.;
}

/// Interval at which the live command duration counter repaints.
const LIVE_COMMAND_DURATION_REPAINT_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Default)]
pub struct ControlMasterErrorBannerState {
    /// Whether or not the control master error banner is currently visible to
    /// the user.
    pub is_open: bool,
    /// The session ID where the error occurred.  This is used to avoid making
    /// additional requests to check for control master errors if we've already
    /// showed the user the banner for this particular session.
    pub associated_session_id: Option<SessionId>,
}

/// Closed => No need for an error banner
/// Triggered => The banner is not open, but should be
/// Open => The banner error is currently open
#[derive(Default)]
pub enum NotificationsErrorBannerType {
    #[default]
    Closed,
    Triggered,
    Open {
        state: NotificationsErrorBannerState,
    },
}

#[derive(Default)]
/// Describes the current state of the notifications error banner
pub struct NotificationsErrorBanner {
    /// The error details
    pub error: Option<NotificationSendError>,
    /// The current state of the error banner (is it open or not)
    pub banner_type: NotificationsErrorBannerType,
}

#[derive(Debug, Clone)]
pub struct BlockNotification {
    pub title: String,
    pub body: String,
}

/// The reason for sending/discovering the notification
#[derive(Copy, Clone, Debug, Serialize)]
pub enum NotificationsTrigger {
    LongRunningCommand(bool /* command_succeeded */, Duration),
    AgentTaskCompleted(bool /* task_succeeded */),
    NeedsAttention,
    /// TODO: Remove this once desktop notifs are unflagged.
    PasswordPrompt,
}

impl NotificationsTrigger {
    pub fn discovery_banner_copy(&self) -> &'static str {
        match self {
            NotificationsTrigger::LongRunningCommand(..) => {
                "Warp can notify you when long-running commands finish."
            }
            NotificationsTrigger::AgentTaskCompleted(..) => {
                "Warp can notify you when an agent finishes responding."
            }
            NotificationsTrigger::NeedsAttention => {
                "Warp can notify you when a command or agent needs your attention."
            }
            NotificationsTrigger::PasswordPrompt => {
                "Warp can notify you when you're prompted to enter a password."
            }
        }
    }

    /// Notifications have the following format
    /// - title: "'{start_of_command}...' {trigger_specific_details}"
    /// - body: "{additional_context} ...{end_of_output}"
    ///
    /// For the command, we show the prefix (if not the whole command) since the user
    /// will likely be able to identify the command more easily by its prefix
    /// e.g. 'ssh user@...' vs '...nux.a.b.com'
    ///
    /// For the output, we show the suffix (if not the whole output) since
    /// the end of the output is what the user likely missed when the terminal
    /// wasn't focused.
    ///
    /// Note: we trim the ends of commands and outputs to remove whitespace
    /// which cause unpleasing gaps in the MacOS notifications.
    pub fn create_notification_content(
        &self,
        command: String,
        output: String,
    ) -> BlockNotification {
        use NotificationsTrigger::*;

        let (title_suffix, body_prefix) = match self {
            LongRunningCommand(command_succeeded, block_duration) => {
                let status = if *command_succeeded {
                    "finished"
                } else {
                    "failed"
                };

                let duration_seconds = block_duration.as_secs_f32();
                let duration_seconds = if duration_seconds >= 1. {
                    format!("{}", duration_seconds.round() as usize)
                } else {
                    format!("{duration_seconds:.1}")
                };

                (
                    format!(" {status} after {duration_seconds}s"),
                    "Latest output: ".to_string(),
                )
            }
            AgentTaskCompleted(command_succeeded) => {
                if *command_succeeded {
                    (" finished".to_string(), "Latest output: ".to_string())
                } else {
                    (" failed".to_string(), "Error: ".to_string())
                }
            }
            NotificationsTrigger::NeedsAttention => (" blocked".to_string(), "".to_string()),
            PasswordPrompt => (
                " is waiting for a password".to_string(),
                "Latest output: ".to_string(),
            ),
        };

        // Get rid of newlines in the command and output because it causes the
        // content of the MacOS notification to appear cutoff or janky.
        let command = command.replace('\n', "\\n");
        let output = output.replace('\n', " ");

        // TITLE

        // Trim off any whitespace from the beginning of the command
        let base_command = command.trim_start();
        let base_command_char_len = base_command.chars().count();

        // Reduce the max character count of the command by 2 for the surrounding quotes
        let title_prefix_max_char_length =
            UserNotification::MAX_TITLE_LENGTH - title_suffix.chars().count() - 2;

        let title_prefix = if title_prefix_max_char_length >= base_command_char_len {
            // The command fits entirely within the title so we can use it as is
            format!("'{}'", base_command.trim_end())
        } else {
            // Otherwise, the command doesn't fit and we need to take the first
            // few characters (minus 3 for the ellipsis) to show
            let end = base_command
                .chars()
                .take(title_prefix_max_char_length - 3)
                .map(|c| c.len_utf8())
                .sum();
            format!("'{}...'", base_command[..end].trim_end())
        };

        // BODY

        // Trim any whitespace off the end of the output
        let base_output = output.trim_end();
        let base_output_char_len = base_output.chars().count();

        let body_suffix_max_char_length =
            UserNotification::MAX_BODY_LENGTH - body_prefix.chars().count();

        let body_suffix = if body_suffix_max_char_length >= base_output_char_len {
            // The output fits entirely within the body so we can use it as is
            base_output.trim_start().to_string()
        } else {
            // Otherwise, the output doesn't fit and we need to take the last
            // few characters (minus 3 for the ellipsis) to show
            let start: usize = base_output.len()
                - base_output
                    .chars()
                    .rev()
                    .take(body_suffix_max_char_length - 3)
                    .map(|c| c.len_utf8())
                    .sum::<usize>();
            format!("...{}", base_output[start..].trim_start())
        };

        BlockNotification {
            title: format!("{title_prefix}{title_suffix}"),
            body: format!("{body_prefix}{body_suffix}"),
        }
    }
}

/// Closed => There is no need for a notifications discovery banner right now
/// Triggered => There is some reason to show the discovery banner, but it's not open yet.
///              For example, the discovery banner for password notifications won't be open
///              till the block completes, but the trigger is non-None
/// Open => The discovery banner is currently open
#[derive(Default)]
pub enum NotificationsDiscoveryBanner {
    #[default]
    Unset,
    Closed,
    Triggered(NotificationsTrigger),
    Open {
        trigger: NotificationsTrigger,
        // Track the request outcome to determine messaging in the banner.
        // None means that the request was not yet responded to.
        request_outcome: Option<RequestPermissionsOutcome>,
        state: NotificationsDiscoveryBannerState,
    },
}

struct ShellProcessTerminatedBanner {
    banner_id: InlineBannerId,
    was_premature_termination: bool,
}

#[derive(Debug, Clone)]
pub enum AgentModePromptSuggestion {
    Success(PromptSuggestion),
    None,
    Error,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptSuggestion {
    pub id: String,

    /// The query that is displayed in the Prompt Suggestion chip to the user.
    /// If this is None, we default to using the prompt itself as the label.
    pub label: Option<String>,

    /// The prompt that is used as the input to Agent Mode.
    pub prompt: String,

    /// If this is some, we eagerly pre-fetch the Agent Mode response for this query.
    pub coding_query_context: Option<Vec<FileLocations>>,

    /// If this is a static prompt suggestion, we store the name of the suggestion type here.
    pub static_prompt_suggestion_name: Option<String>,

    // Whether or not accepting this prompt suggestion should start a new conversation or continue
    // the existing one. Only applies when in agent view; in terminal view, prompt suggestions
    // always start a new conversation.
    pub should_start_new_conversation: bool,
}

impl PromptSuggestion {
    pub fn is_coding_query(&self) -> bool {
        self.coding_query_context.is_some()
    }

    /// Returns specified label for Prompt Suggestion if it exists, otherwise returns the query
    /// (which is considered to be the "default" label).
    pub fn label(&self) -> &String {
        self.label.as_ref().unwrap_or(&self.prompt)
    }

    pub fn is_static_prompt_suggestion(&self) -> bool {
        self.static_prompt_suggestion_name.is_some()
    }
}

/// A unique identifier for an inline banner.
pub type InlineBannerId = usize;

/// Type of inline banner - determines behavior like visibility in agent view.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum InlineBannerType {
    NotificationsDiscovery,
    NotificationsError,
    Ssh,
    PromptSuggestions,
    AliasExpansion,
    SharedSessionStart,
    SharedSessionEnd,
    ShellProcessTerminated,
    OpenInWarp,
    VimMode,
    CodebaseIndexSpeedbump,
    AgentModeSetup,
    AnonymousUserAISignUp,
    AwsBedrockLogin,
    AwsCliNotInstalled,
}

impl InlineBannerType {
    /// Returns whether this banner type should be visible when agent view is active.
    /// Exhaustive match ensures new banner types must define their visibility.
    pub fn is_visible_in_agent_view(&self) -> bool {
        match self {
            // Agent-related banners: visible in agent view
            Self::PromptSuggestions
            | Self::CodebaseIndexSpeedbump
            | Self::AgentModeSetup
            | Self::AnonymousUserAISignUp
            | Self::AwsBedrockLogin
            | Self::AwsCliNotInstalled => true,
            // Terminal-context banners: hidden in agent view
            Self::NotificationsDiscovery
            | Self::NotificationsError
            | Self::Ssh
            | Self::AliasExpansion
            | Self::SharedSessionStart
            | Self::SharedSessionEnd
            | Self::ShellProcessTerminated
            | Self::OpenInWarp
            | Self::VimMode => false,
        }
    }
}

/// An inline banner with its unique ID and type metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct InlineBannerItem {
    pub id: InlineBannerId,
    pub banner_type: InlineBannerType,
}

impl InlineBannerItem {
    pub fn new(id: InlineBannerId, banner_type: InlineBannerType) -> Self {
        Self { id, banner_type }
    }
}

/// A unique identifier for a subshell separator.
pub type SeparatorId = usize;

#[derive(Default)]
struct InlineBannersState {
    /// The ID for the next inline banner to be created.
    next_banner_id: InlineBannerId,

    /// State for the different notification banners.
    notifications_discovery_banner: NotificationsDiscoveryBanner,
    notifications_error_banner: NotificationsErrorBanner,

    /// A mapping from banner ID to state information for all SSH banners in
    /// this view.
    ssh_banners: HashMap<InlineBannerId, SSHBannerState>,

    prompt_suggestions_banner: Option<PromptSuggestionBannerState>,

    alias_expansion_banner: AliasExpansionBanner,

    shared_session_banner_state: SharedSessionBanners,

    /// Information for a banner which notifies the user that the
    /// shell process has terminated, or None if there is no
    /// banner to display.
    shell_process_terminated_banner: Option<ShellProcessTerminatedBanner>,

    open_in_warp_banner: Option<OpenInWarpBannerState>,

    vim_banner_state: Option<VimModeBannerState>,

    codebase_index_speedbump_banner: Option<CodebaseIndexSpeedbumpBannerState>,

    agent_setup_speedbump_banner: Option<AgentModeSetupSpeedbumpBannerState>,

    anonymous_user_ai_sign_up_banner: Option<AnonymousUserAISignUpBannerState>,

    aws_bedrock_login_banner: Option<AwsBedrockLoginBannerState>,

    aws_cli_not_installed_banner: Option<AwsCliNotInstalledBannerState>,
}

impl InlineBannersState {
    /// Returns the ID to assign to the next inline banner.
    fn next_banner_id(&mut self) -> InlineBannerId {
        let next_id = self.next_banner_id;
        self.next_banner_id += 1;
        next_id
    }

    /// Returns the ID of the last inline banner inserted.
    #[allow(dead_code)]
    fn last_banner_id(&self) -> Option<InlineBannerId> {
        #[allow(clippy::unnecessary_lazy_evaluations)]
        (self.next_banner_id > 0).then(|| self.next_banner_id - 1)
    }
}

/// Banners that we include in the blocklist to delimit
/// the start and endpoints of the shared session status, if any.
#[derive(Copy, Clone, Default)]
pub enum SharedSessionBanners {
    /// There aren't any shared session banners.
    #[default]
    None,

    /// This session is currently being shared, so
    /// we only have a started banner.
    ActiveShare {
        started_banner_id: InlineBannerId,
        started_at: DateTime<Local>,
        is_remote_control: bool,
    },

    /// This session is not actively being shared, but
    /// it was shared at some point, so we have start and
    /// end banners.
    LastShared {
        started_banner_id: InlineBannerId,
        started_at: DateTime<Local>,
        is_remote_control: bool,

        ended_banner_id: InlineBannerId,
        ended_at: DateTime<Local>,
    },
}

/// Helper struct for creating SizeUpdates.
#[derive(Debug)]
struct SizeUpdateBuilder {
    /// The reason for the size update.
    update_reason: SizeUpdateReason,

    /// The last size info prior to the update.
    last_size: SizeInfo,

    /// The new pane size in pixels.
    new_pane_size_px: Vector2F,
}

impl SizeUpdateBuilder {
    fn for_refresh(last_size: SizeInfo) -> Self {
        // Refreshing doesn't actually change pane size or content element size.
        Self {
            update_reason: SizeUpdateReason::Refresh,
            last_size,
            new_pane_size_px: last_size.pane_size_px(),
        }
    }

    fn for_shared_session_update(last_size: SizeInfo, num_rows: usize, num_cols: usize) -> Self {
        // Shared session updates don't change the actual pane / content sizes.
        Self {
            update_reason: SizeUpdateReason::SharerSizeChanged { num_rows, num_cols },
            last_size,
            new_pane_size_px: last_size.pane_size_px(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn for_viewer_size_report(last_size: SizeInfo, num_rows: usize, num_cols: usize) -> Self {
        // Viewer size reports don't change the sharer's actual pane size.
        Self {
            update_reason: SizeUpdateReason::ViewerSizeReported { num_rows, num_cols },
            last_size,
            new_pane_size_px: last_size.pane_size_px(),
        }
    }

    fn after_layout(last_size: SizeInfo, new_pane_size_px: Vector2F) -> Self {
        Self {
            update_reason: SizeUpdateReason::AfterLayout,
            last_size,
            new_pane_size_px,
        }
    }

    fn build(self, view: &TerminalView, ctx: &ViewContext<TerminalView>) -> SizeUpdate {
        let appearance = view.appearance(ctx);
        let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
        let model = view.model.lock();

        let new_size = create_size_info(
            self.new_pane_size_px,
            &model,
            view.sessions.as_ref(ctx),
            ctx.font_cache(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
            appearance.line_height_ratio(),
            ctx,
        );

        // Capture the pane-computed natural size before shared session adjustments.
        let natural_rows = new_size.rows;
        let natural_cols = new_size.columns;

        let new_size = match self.update_reason {
            SizeUpdateReason::SharerSizeChanged { num_rows, num_cols } => {
                // For a shared session viewer, we want to use the larger
                // of our own size and the sharer's size. So we adjust
                // the number of rows and columns to be the greater
                // of our own and the sharer's.
                let rows = num_rows.max(new_size.rows);
                let cols = num_cols.max(new_size.columns);
                new_size.with_rows_and_columns(rows, cols)
            }
            SizeUpdateReason::ViewerSizeReported { num_rows, num_cols } => {
                // Use the viewer's reported size directly so the PTY
                // matches the viewer's viewport (floored at 1).
                new_size.with_rows_and_columns(num_rows.max(1), num_cols.max(1))
            }
            _ => {
                // For a shared session viewer, we want to use the larger
                // of our own size and the sharer's size.
                // However, if the viewer is actively reporting its size to the sharer
                // (viewer-driven sizing), skip the MAX — the PTY is already at our size.
                if let Some(Viewer {
                    sharer_size,
                    last_reported_natural_size,
                    ..
                }) = view.shared_session_viewer()
                {
                    if last_reported_natural_size.is_some() {
                        // Viewer-driven sizing is active; use our own natural size.
                        new_size
                    } else if let Some(size) = sharer_size {
                        let rows = size.num_rows.max(new_size.rows);
                        let cols = size.num_cols.max(new_size.columns);
                        new_size.with_rows_and_columns(rows, cols)
                    } else {
                        new_size
                    }
                } else if let Some((viewer_rows, viewer_cols)) = view.active_viewer_driven_size {
                    // Sharer honoring a viewer's reported size: use the viewer's
                    // dimensions so AfterLayout doesn't override back to the sharer's natural size.
                    new_size.with_rows_and_columns(viewer_rows.max(1), viewer_cols.max(1))
                } else {
                    new_size
                }
            }
        };

        // Adjust the gap size to maintain the model invariant that the height of the
        // gap + all block_heights after the gap equals the height of the current
        // space in which to render the blocklist.  Note that we also need to run this
        // same logic when the input mode switches to Waterfall.
        let viewport = view.viewport_state(model.block_list(), input_mode, ctx);
        let new_gap_height = match (input_mode, model.block_list().active_gap()) {
            (InputMode::Waterfall, Some(gap)) => {
                let block_list_height_without_gap =
                    model.block_list().block_heights().summary().height - gap.height();
                let max_scroll_top = viewport.max_scroll_top_in_lines();
                let input_id = view.input.as_ref(ctx).save_position_id();
                let mut input_height =
                    element_size_at_last_frame(input_id.as_str(), ctx.window_id(), ctx)
                        .map_or(0., |r| r.y())
                        .into_pixels()
                        .to_lines(new_size.cell_height_px());

                // Here there be dragons!!!
                //
                // When the inline menu is open in waterfall mode, we apply a paint-time
                // translation of the blocklist element to simulate the blocklist 'sliding'
                // upwards, which allows the inline menu to be rendered beneath the blocklist,
                // but preserves the input's vertical position.
                //
                // The fact that this is paint-time is important - it minimizes the surface area of
                // logic that needs to even be aware of the inline menu visibility.
                //
                // However, it also means that the blocklist datamodel (heights in the sumtree)
                // needs to be totally decoupled from inline menu visibility. This is the one place
                // where the rendered positioning/size of the input element (which includes the
                // inline menu) can actually affect sumtree heights -- when we recompute the 'gap'
                // size in waterfall mode, which depends on the rendered input element size.
                //
                // Thus, when there is a gap and the inline menu is open, the gap should not
                // account for the inline menu being open - it should remain the same size, and
                // we explicitly subtract the height of the inline menu from the height of the input
                // we use to determine the new gap height.
                input_height -= view
                    .inline_menu_positioner
                    .as_ref(ctx)
                    .blocklist_top_inset_when_in_waterfall_mode(ctx)
                    .unwrap_or_default()
                    .to_lines(new_size.cell_height_px());

                let new_height = max_scroll_top
                    + new_size
                        .pane_height_px()
                        .to_lines(new_size.cell_height_px())
                    - block_list_height_without_gap
                    - input_height;
                (!heights_approx_eq(new_height, gap.height())).then_some(new_height)
            }
            (_, _) => None,
        };

        SizeUpdate {
            update_reason: self.update_reason,
            last_size: self.last_size,
            new_size,
            new_gap_height,
            natural_rows,
            natural_cols,
        }
    }
}

struct FindLinkArg {
    position: WithinModel<Point>,
    from_editor: TerminalEditor,
}

#[derive(Debug, Clone, Copy)]
pub enum TerminalEditor {
    Yes,
    No,
}

/// Different modes for how we consider a block to be "visible"
#[derive(Debug, Clone, Copy)]
pub enum BlockVisibilityMode {
    /// A block is visible if its top is on screen
    TopOfBlockVisible,

    /// A block is visible if its bottom is on screen
    BottomOfBlockVisible,
}

#[derive(Clone)]
pub enum ContextMenuAction {
    InsertSelectedText,
    CopySelectedText,
    CopyUrl {
        url_content: String,
    },
    CopyBlocks,
    CopyBlockCommands,
    CopyBlockOutputs,
    CopyBlockFilteredOutputs,
    OpenShareBlockModal {
        block_index: BlockIndex,
    },
    FindWithinBlock,
    ToggleBookmark,
    ScrollToBottomOfBlock,
    ScrollToTopOfBlock,
    CopyPrompt {
        position: PromptPosition,
        part: PromptPart,
    },
    CopyRprompt,
    EditPrompt,
    EditAgentToolbar,
    EditCLIAgentToolbar,
    /// Ask AI about the current context. Handled by blocklist AI if its feature flag is enabled and
    /// the AI assistant panel otherwise.
    AskAI(AskAISource),
    OpenWorkflowModal,
    CopyAIDebuggingLink {
        conversation_token: ServerConversationToken,
        request_id: Option<ServerOutputId>,
    },
    CopyExternalDebuggingId {
        request_id: Option<ServerOutputId>,
        conversation_id: ServerConversationToken,
    },
    CopyConversationId {
        conversation_id: ServerConversationToken,
    },
    CopyServerRequestId {
        request_id: ServerConversationToken,
    },
    // Copy the share link for a conversation in the blocklist.
    CopyConversationShareLink {
        conversation_id: AIConversationId,
    },
    // Copy the text of a conversation in the blocklist.
    CopyConversationText {
        conversation_id: AIConversationId,
    },
    // Fork a conversation in the blocklist into a new pane.
    ForkAIConversation {
        conversation_id: AIConversationId,
    },
    /// Opens the sharing dialog for a conversation from the AI block context menu
    OpenConversationShareDialog {
        conversation_id: AIConversationId,
    },
    OpenShareSessionModal,
    StopSharing,
    /// Copy the AI block prompt text
    CopyAIBlockQuery {
        ai_block_view_id: EntityId,
    },
    /// Copy the AI block output text
    CopyAIBlockOutput {
        ai_block_view_id: EntityId,
    },
    /// Copy both AI block prompt and output text
    CopyAIBlock {
        ai_block_view_id: EntityId,
    },
    /// Copy the complete AI conversation history
    CopyAIBlockConversation {
        ai_block_view_id: EntityId,
    },
    CopyAgentCommand {
        ai_block_view_id: EntityId,
    },
    CopyAgentGitBranch {
        ai_block_view_id: EntityId,
    },
    /// Fork the AI conversation from the block corresponding to this AI block.
    /// Forks at the query boundary (includes all exchanges up to the next user query).
    ForkAIConversationFromBlock {
        ai_block_view_id: EntityId,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
    },
    /// Fork the AI conversation from the exact exchange that was clicked on.
    ForkAIConversationFromExactExchange {
        ai_block_view_id: EntityId,
        exchange_id: AIAgentExchangeId,
        conversation_id: AIConversationId,
    },
    /// Save the AI block prompt as an agent mode workflow (saved prompt)
    SavePromptAsAgentModeWorkflow {
        ai_block_view_id: EntityId,
    },
}

#[derive(Clone)]
pub enum InputContextMenuAction {
    CutSelectedText,
    CopySelectedText,
    SelectAll,
    Paste,
    ShowCommandSearch,
    ShowAICommandSearch,
    AskWarpAI,
    SaveAsWorkflow,
    ToggleInputHintText,
}

/// Where a user's question for AI originated. Handled by blocklist AI if the feature flag is
/// enabled and the AI Assistant panel otherwise.
#[derive(Clone)]
pub enum AskAISource {
    Block(BlockIndex),
    LastBlock,
    /// The source is some selected text or selected block, but we're not yet sure which.
    /// There should never be any cases where both are simultaneously selected.
    SelectedBlockOrText,
    /// Question for block list AI about text selected form the terminal or input.
    SelectedInputText,
    SelectedTerminalText,
    /// Question for block list AI about block(s).
    SelectedBlocks,
}

// Manually implementing Debug to avoid leaking sensitive information in logs
impl fmt::Debug for ContextMenuAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use ContextMenuAction::*;

        match self {
            InsertSelectedText => f.write_str("InsertSelectedText"),
            CopySelectedText => f.write_str("CopySelectedText"),
            CopyBlocks => f.write_str("CopyBlocks"),
            CopyBlockCommands => f.write_str("CopyBlockCommands"),
            CopyBlockOutputs => f.write_str("CopyBlockOutputs"),
            OpenShareBlockModal { block_index } => {
                write!(f, "OpenShareModal {{ block_index: {block_index} }}")
            }
            FindWithinBlock => f.write_str("FindWithinBlock"),
            ScrollToBottomOfBlock => f.write_str("ScrollToBottomOfBlock"),
            ScrollToTopOfBlock => f.write_str("ScrollToTopOfBlock"),
            ToggleBookmark => f.write_str("BookmarkBlock"),
            CopyPrompt { position, part } => {
                write!(f, "CopyPrompt {{ position: {position:?}, part: {part:?} }}")
            }
            CopyRprompt => f.write_str("CopyRprompt"),
            // CopyUrl's debug output is limited, since the URLs come from command output
            CopyUrl { .. } => f.write_str("CopyUrl"),
            EditPrompt => f.write_str("EditPrompt"),
            EditAgentToolbar => f.write_str("EditAgentToolbar"),
            EditCLIAgentToolbar => f.write_str("EditCLIAgentToolbar"),
            AskAI(_) => f.write_str("AskAIAssistant"),
            OpenWorkflowModal => f.write_str("OpenWorkflowModal"),
            OpenShareSessionModal => f.write_str("OpenShareSessionModal"),
            CopyBlockFilteredOutputs => f.write_str("CopyBlockFilteredOutput"),
            StopSharing => f.write_str("StopSharing"),
            CopyAIDebuggingLink { .. } => f.write_str("CopyAIDebuggingLink"),
            CopyAIBlockQuery { .. } => f.write_str("CopyAIBlockPrompt"),
            CopyAIBlockOutput { .. } => f.write_str("CopyAIBlockOutput"),
            CopyAIBlock { .. } => f.write_str("CopyAIBlockBoth"),
            CopyAIBlockConversation { .. } => f.write_str("CopyAIBlockConversation"),
            CopyAgentCommand { .. } => f.write_str("CopyAgentCommand"),
            CopyAgentGitBranch { .. } => f.write_str("CopyAgentGitBranch"),
            CopyExternalDebuggingId { .. } => f.write_str("CopyExternalDebuggingId"),
            CopyConversationId { .. } => f.write_str("CopyConversationId"),
            CopyServerRequestId { .. } => f.write_str("CopyServerRequestId"),
            CopyConversationShareLink { .. } => f.write_str("CopyConversationShareLink"),
            CopyConversationText { .. } => f.write_str("CopyConversationText"),
            ForkAIConversation { .. } => f.write_str("ForkAIConversation"),
            OpenConversationShareDialog { .. } => f.write_str("OpenConversationShareDialog"),
            ForkAIConversationFromBlock { .. } => f.write_str("ForkAIConversationFromBlock"),
            ForkAIConversationFromExactExchange { .. } => {
                f.write_str("ForkAIConversationFromExactExchange")
            }
            SavePromptAsAgentModeWorkflow { .. } => f.write_str("SavePromptAsAgentModeWorkflow"),
        }
    }
}

// Manually implementing Debug to avoid leaking sensitive information in logs
impl fmt::Debug for InputContextMenuAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use InputContextMenuAction::*;

        match self {
            CutSelectedText => f.write_str("CutSelectedText"),
            CopySelectedText => f.write_str("CopySelectedText"),
            SelectAll => f.write_str("SelectAll"),
            Paste => f.write_str("Paste"),
            ShowCommandSearch => f.write_str("CommandSearch"),
            ShowAICommandSearch => f.write_str("AICommandSearch"),
            AskWarpAI => f.write_str("AskWarpAI"),
            SaveAsWorkflow => f.write_str("SaveAsWorkflow"),
            ToggleInputHintText => f.write_str("ToggleInputHintText"),
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum PromptPosition {
    Block(BlockIndex),
    Input,
}

impl PromptPosition {
    fn block<'a>(&self, model: &'a TerminalModel) -> Option<&'a Block> {
        match self {
            PromptPosition::Block(block_index) => model.block_list().block_at(*block_index),
            PromptPosition::Input => Some(model.block_list().active_block()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum PromptPart {
    EntirePrompt,
    CondaContext,
    Pwd,
    GitBranch,
    VirtualEnv,
    ContextChip(ContextChipKind),
}

/// Arg for calculating the next bookmark position.
struct IndicatorPositionArg {
    remaining_indicator_count: usize,
    /// Previous rendered indicator top.
    previous_indicator_top: Pixels,
}

impl IndicatorPositionArg {
    fn next_indicator_top(
        &mut self,
        block_start: Lines,
        total_block_height: Lines,
        content_height: Pixels,
    ) -> Pixels {
        self.remaining_indicator_count -= 1;

        // Total height an indicator will take (its height + minimum gap between indicators).
        let indicator_height = BOOKMARK_INDICATOR_HEIGHT + BOOKMARK_MIN_GAP;
        let mut top = (content_height * (block_start / total_block_height).as_f64().into_pixels())
            .max(self.previous_indicator_top + indicator_height.into_pixels());

        let remaining_space = content_height - (top + indicator_height.into_pixels());
        let remaining_indicator_required_space =
            (self.remaining_indicator_count as f32 * indicator_height).into_pixels();

        // Only move the indicator up if there is not enough space for the remaining
        // indicators AND the new position won't cause the indicators' ordering to change
        // or result in a negative top.
        if remaining_space < remaining_indicator_required_space
            && content_height - remaining_indicator_required_space > self.previous_indicator_top
        {
            top = content_height - remaining_indicator_required_space;
        }

        self.previous_indicator_top = top;
        top
    }
}

#[derive(Clone)]
pub struct ExecuteAIRequestedCommandEvent {
    pub requested_command_id: AIAgentActionId,
    pub command: String,
    pub shell_type: ShellType,
}

#[derive(Clone)]
pub struct ExecuteCommandEvent {
    pub command: String,
    pub session_id: SessionId,

    /// If the command was executed from a [`CloudWorkflow`], pass its ID here.
    pub workflow_id: Option<SyncId>,
    /// If the command was executed from a [`CloudWorkflow`] or WorkflowType::Local, store the
    /// templated command here.
    pub workflow_command: Option<String>,

    /// `true` if the executed command should be added to session history.
    pub should_add_command_to_history: bool,

    pub source: CommandExecutionSource,
}

/// Actions that can be taken on a passive code diff via the input editor.
#[derive(Clone, Debug)]
pub enum CodeDiffAction {
    Accept,
    Reject,
    Edit,
    ScrollToExpand,
}

pub enum Event {
    AppStateChanged,
    Escape,
    Exited,
    BlockListCleared,
    ShareModalOpened(BlockIndex),
    SendNotification(BlockNotification),
    BlockCompleted {
        block: Arc<SerializedBlock>,
        is_local: bool,
    },
    Pane(PaneEvent),
    OpenSettings(SettingsSection),
    AskAIAssistant(AskAIType),
    /// Event propagates terminal inputs up to the workspace,
    /// to be processed on the way back down through the view hierarchy.
    SyncInput(SyncEvent),
    /// Event used to propagate a state change for one of the terminal views
    /// inside this pane group.
    TerminalViewStateChanged,
    ShowCommandSearch(CommandSearchOptions),
    // Tell the pane group to open the workflow modal.
    OpenWorkflowModalWithCommand(String),
    // Tell the pane group to open the workflow modal with an existing cloud workflow.
    OpenWorkflowModalWithCloudWorkflow(SyncId),
    // Tell the pane group to open the workflow modal with an unsaved workflow.
    OpenWorkflowModalWithTemporary(Box<Workflow>),
    OpenWarpDriveObjectInPane(ObjectUid),
    OpenSuggestedAgentModeWorkflowModal {
        workflow_and_id: SuggestedAgentModeWorkflowAndId,
    },
    OpenSuggestedRuleDialog {
        rule_and_id: SuggestedRuleAndId,
    },
    OpenAIFactCollection {
        /// If set, open the fact collection to the specific rule.
        sync_id: Option<SyncId>,
    },
    ToggleAIDocumentPane {
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
    },
    /// Closes all visible AI document panes without opening a new one.
    HideAIDocumentPanes,
    /// Opens an AI document pane.
    /// When `is_auto_open` is true, subject to conditions to check if auto opening is acceptable.
    /// When `is_auto_open` is false (user-triggered), always opens unconditionally.
    OpenAIDocumentPane {
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
        is_auto_open: bool,
    },
    OpenPromptEditor,
    OpenAgentToolbarEditor,
    OpenCLIAgentToolbarEditor,
    SummarizationCancelDialogToggled {
        is_open: bool,
    },
    EnvironmentSetupModeSelectorToggled {
        is_open: bool,
    },
    AuthSecretDeleteConfirmationDialogToggled {
        is_open: bool,
    },
    CtrlD,
    ShutdownPty,
    // TODO: break this event down into higher-level events that hide the
    // `bytes` detail from the view.
    WriteBytesToPty {
        bytes: Cow<'static, [u8]>,
    },
    WriteAgentInputToPty {
        bytes: Cow<'static, [u8]>,
        mode: AIAgentPtyWriteMode,
    },
    Resize {
        size_update: SizeUpdate,
    },
    ExecuteCommand(ExecuteCommandEvent),
    BlockStarted {
        is_for_in_band_command: bool,
    },
    /// Tell the pane group to open a file within Warp.
    OpenFileInWarp {
        path: PathBuf,
        /// The session that the file belongs to.
        session: Arc<Session>,
    },
    #[cfg(feature = "local_fs")]
    OpenCodeInWarp {
        source: CodeSource,
        layout: EditorLayout,
    },
    #[cfg(feature = "local_fs")]
    PreviewCodeInWarp {
        source: CodeSource,
    },
    OpenCodeDiff {
        view: ViewHandle<CodeDiffView>,
    },
    OpenCodeReviewPane(CodeReviewPanelArg),
    ToggleCodeReviewPane(CodeReviewPanelArg),
    InsertCodeReviewComments {
        repo_path: LocalOrRemotePath,
        comments: Vec<PendingImportedReviewComment>,
        diff_mode: DiffMode,
        open_code_review: Option<CodeReviewPanelArg>,
    },
    OpenCodeReviewPaneAndScrollToComment {
        open_code_review: CodeReviewPanelArg,
        comment: AttachedReviewComment,
        diff_mode: DiffMode,
    },
    ImportAllCodeReviewComments {
        open_code_review: CodeReviewPanelArg,
        comments: Vec<AttachedReviewComment>,
        diff_mode: DiffMode,
    },
    StartSharingCurrentSession {
        scrollback_type: SharedSessionScrollbackType,
        source: SharedSessionSource,
    },
    EstablishedSharedSession {
        session_id: session_sharing_protocol::common::SessionId,
    },
    FailedToShareSession {
        reason: String,
        cause: Option<Arc<anyhow::Error>>,
    },
    RejoinCurrentSession,
    StopSharingCurrentSession {
        reason: SessionEndedReason,
    },
    CloseRequested,
    OpenShareSessionModal {
        open_source: SharedSessionActionSource,
    },
    OpenShareSessionDeniedModal,
    /// Used to focus and bring this session to the foreground.
    FocusSession,
    /// Emitted when the onboarding init flow completes.
    OnboardingInitCompleted,
    /// Emitted when the guided onboarding tutorial callout is completed or dismissed.
    OnboardingTutorialCompleted,
    SelectedBlocksChanged,
    SelectedTextChanged,
    UpdateSessionLinkPermissions {
        role: Option<Role>,
    },
    UpdateSessionTeamPermissions {
        role: Option<Role>,
        team_uid: String,
    },
    /// Emitted when a shared session sharer updates a viewer's role and
    /// needs to notify the server of a role change.
    UpdateRole {
        participant_id: ParticipantId,
        role: Role,
    },
    UpdateUserRole {
        user_uid: UserUid,
        role: Role,
    },
    UpdatePendingUserRole {
        email: String,
        role: Role,
    },
    AddGuests {
        emails: Vec<String>,
        role: Role,
    },
    RemoveGuest {
        user_uid: UserUid,
    },
    RemovePendingGuest {
        email: String,
    },
    MakeAllParticipantsReaders {
        reason: RoleUpdateReason,
    },
    RequestSharedSessionRole(Role),
    /// A viewer in a shared session is requesting to send an agent prompt.
    SendAgentPrompt {
        server_conversation_token: Option<SessionSharingServerConversationToken>,
        prompt: String,
        attachments: Vec<AgentAttachment>,
    },
    /// A viewer in a shared session is requesting to cancel the active agent conversation.
    CancelSharedSessionConversation {
        server_conversation_token: SessionSharingServerConversationToken,
    },
    /// The viewer is reporting its terminal size for viewer-driven PTY sizing.
    ReportViewerTerminalSize {
        window_size: SessionSharingWindowSize,
    },
    /// The input editor was locally edited and
    /// peers should be notified, if applicable.
    InputEditorUpdated {
        /// The block ID associated to the buffer that
        /// these operations were made in.
        block_id: BlockId,

        /// The CRDT-compliant operations.
        operations: Rc<Vec<CrdtOperation>>,
    },
    /// Emitted when a shared session participant tries to
    /// change a role. `source` dictates how the modal is rendered,
    /// and what fields are needed
    OpenSharedSessionRoleChangeModal {
        source: RoleChangeOpenSource,
    },
    CloseSharedSessionRoleChangeModal(RoleChangeCloseSource),
    RoleRequestInFlight {
        role_request_id: RoleRequestId,
    },
    CancelRoleRequest(RoleRequestId),
    RoleRequestCancelled(RoleRequestId),
    RespondToRoleRequest {
        participant_id: ParticipantId,
        role_request_id: RoleRequestId,
        response: RoleRequestResponse,
    },
    /// Emitted when a pending command (e.g. tab config setup commands) has
    /// been submitted and its block has completed.
    PendingCommandCompleted,
    SessionBootstrapped,
    AnonymousUserSignup,
    ShellSpawned(ShellType),

    /// This terminal pane has initiated a file upload to a remote host.
    CopyFileToRemote {
        command: String,
        upload_id: FileUploadId,
    },
    /// This terminal pane is taking care of a file upload to a remote host
    /// and requires a password.
    FileUploadPasswordPending,
    /// This terminal pane was taking care of a file upload to a remote host
    /// and just finished a block.
    FileUploadFinished(ExitCode),
    /// Open the terminal pane that is taking care of a file upload to
    /// this pane's remote host.
    OpenFileUploadSession(FileUploadId),
    /// Terminate the session that took care of a file upload for this pane's
    /// remote host.
    TerminateFileUploadSession(FileUploadId),
    RunNativeShellCompletions {
        buffer_text: String,
        results_tx: async_channel::Sender<Vec<ShellCompletion>>,
    },
    /// Emitted when the user clicks "install" in the SSH remote-server choice block.
    RemoteServerInstallRequested {
        session_id: SessionId,
    },
    /// Emitted when the user clicks "skip" in the SSH remote-server choice block.
    RemoteServerSkipRequested {
        session_id: SessionId,
    },
    SignupAnonymousUser {
        entrypoint: AnonymousUserSignupEntrypoint,
    },

    OpenThemeChooser,
    OpenConversationHistory,
    OpenMCPSettingsPage {
        page: Option<MCPServersSettingsPage>,
    },
    OpenAddRulePane,
    OpenRulesPane,
    OpenAddPromptPane {
        /// The initial prompt body content.
        initial_content: Option<String>,
    },
    OpenEnvironmentManagementPane,
    OpenFilesPalette {
        source: PaletteSource,
    },
    #[cfg(feature = "local_fs")]
    OpenFileWithTarget {
        path: PathBuf,
        target: FileTarget,
        line_col: Option<LineAndColumnArg>,
    },
    /// Emitted when a file in the file tree is renamed.
    #[cfg(feature = "local_fs")]
    FileRenamed {
        old_path: PathBuf,
        new_path: PathBuf,
    },
    /// Emitted when a file in the file tree is deleted.
    #[cfg(feature = "local_fs")]
    FileDeleted {
        path: PathBuf,
    },
    /// Toggle the left panel to a specific view
    ToggleLeftPanel {
        target_view: LeftPanelTargetView,
        force_open: bool,
    },
    SlowBootstrap,
    OpenAgentProfileEditor {
        profile_id: ClientProfileId,
    },
    OpenAutoReloadModal {
        purchased_credits: i32,
    },
    #[cfg(not(target_family = "wasm"))]
    OpenPluginInstructionsPane(CLIAgent, PluginModalKind),
    ShowToast {
        message: String,
        flavor: ToastFlavor,
    },
    /// Emitted when the agent's interaction state with a long-running command changes.
    LongRunningCommandAgentInteractionStateChanged {
        state: LongRunningCommandAgentInteractionState,
    },
    /// A pluggable notification triggered via OSC 9 or OSC 777 escape sequences.
    /// Used to show an in-app toast notification.
    PluggableNotification {
        title: Option<String>,
        body: String,
    },
    /// Emitted when cloud mode runs should display the cloud-agent capacity/credits modal.
    ShowCloudAgentCapacityModal {
        variant: CloudAgentCapacityModalVariant,
    },
    FreeTierLimitCheckTriggered,
    /// Emitted when the StartAgent executor needs the workspace to create
    /// a new child agent conversation in a split pane. The freshly-created
    /// child conversation id is echoed back to the executor via
    /// [`BlocklistAIHistoryModel::record_new_conversation_request_complete`]
    /// so the executor can disambiguate per-request pendings when multiple
    /// StartAgent requests are in flight in parallel.
    StartAgentConversation(StartAgentRequest),
    /// Emitted when the user clicks a child agent row in the status card to reveal
    /// its hidden pane.
    RevealChildAgent {
        conversation_id: AIConversationId,
    },
    /// Emitted when the user clicks a pill in the orchestration pill bar.
    /// The pane group swaps visibility instead of cloning the conversation.
    SwapPaneToConversation {
        conversation_id: AIConversationId,
    },
    /// Emitted by `OrchestrationViewerModel` when a child of a shared-session
    /// orchestration first reports a `session_id`. The pane group materializes
    /// a dedicated hidden shared-session viewer pane for the child, with its
    /// own `TerminalView`, `BlocklistAIController`, and viewer-side `Network`
    /// joining the child's session. Subsequent pill clicks navigate to the
    /// hidden pane via the existing `SwapPaneToConversation` mechanism.
    EnsureSharedSessionViewerChildPane {
        conversation_id: AIConversationId,
        session_id: session_sharing_protocol::common::SessionId,
    },
    /// Emitted when "Open in new tab" is picked from a child pill's 3-dot menu.
    /// Bubbles up to the workspace to create the new tab.
    OpenChildAgentInNewTab {
        conversation_id: AIConversationId,
    },
    /// Emitted when "Open in new pane" is picked from a child pill's 3-dot menu.
    /// Reuses the existing dedicated child pane to preserve in-flight state.
    OpenChildAgentInNewPane {
        conversation_id: AIConversationId,
    },
    /// Emitted when "Stop agent" is picked from a child pill's 3-dot menu.
    StopAgentConversation {
        conversation_id: AIConversationId,
    },
    /// Emitted when "Kill agent" is picked from a child pill's 3-dot menu.
    KillAgentConversation {
        conversation_id: AIConversationId,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum LeftPanelTargetView {
    FileTree,
    WarpDrive,
}

#[derive(Clone)]
pub struct SyncEvent {
    /// Used to prevent updating the source of the changes.
    /// Note: `StartSyncing` and `StopSyncing` don't use `source_view_id`
    /// because they should be acted on regardless of where
    /// the event originated (e.g., a terminal view should sync itself).
    pub source_view_id: EntityId,
    pub data: SyncInputType,
}

/// Event used to propagate the keyboard events from one terminal to others.
#[derive(Clone)]
pub enum SyncInputType {
    /// Event for when the input editor's buffer contents changed.
    InputEditorContentsChanged {
        /// Note: Using Arc because to make efficient cloning of large string possible
        contents: Arc<String>,
    },
    /// Event to handle user keyboard input to
    /// the alt-screen or long-running commands/
    NonEditorTyped {
        /// Characters the user inputted
        /// Note: Using Arc because to make efficient cloning of large string possible
        chars: Arc<Vec<u8>>,
    },
    /// Event used to to run commands in all synced terminals with
    /// visible input editors.
    RanCommand,
    /// Event used to notify that this terminal should be synced but we don't
    /// need to update its input editor or write to its PTY.
    StartSyncing,
    /// Event tells us we should stop syncing this terminal.
    StopSyncing,
}

#[derive(Debug, Copy, Clone)]
pub enum ContextMenuType {
    /// Opened via right-clicking within any block or using a block's 3-dot menu.
    BlockList { menu_source: BlockListMenuSource },
    /// Opened via right-clicking anywhere on the alt-screen.
    AltScreen { position: Vector2F },
    /// Opened via right-clicking on the input prompt.
    Prompt { position: Vector2F },
    /// Opened via right-clicking on the input box.
    Input { position: Vector2F },

    /// Lists the block(s) or text attached as context to the query represented in the AI block
    /// whose view id is the given [`EntityId`]. The menu is opened by clicking on the attached
    /// context chip inside the AI block.
    AIBlockAttachedContext { ai_block_view_id: EntityId },
    /// Shows the overflow menu with copy options for an AI block. The menu is opened by clicking
    /// on the overflow (three dots) button inside the AI block header.
    AIBlockOverflowMenu { ai_block_view_id: EntityId },
    /// Shows the conversation actions menu for an Agent View entry block.
    AgentViewEntryConversation {
        agent_view_entry_block_id: EntityId,
        position: Vector2F,
    },
}

impl ContextMenuType {
    pub fn origin(&self) -> Option<Vector2F> {
        match self {
            ContextMenuType::BlockList { menu_source } => match menu_source {
                BlockListMenuSource::RegularBlockRightClick {
                    position_in_terminal_view,
                    ..
                } => Some(*position_in_terminal_view),
                BlockListMenuSource::OutsideBlockRightClick {
                    position_in_terminal_view,
                    ..
                } => Some(*position_in_terminal_view),
                // We may be able to get the point from the row/col
                BlockListMenuSource::BlockOverflowButton { .. } => None,
                BlockListMenuSource::BlockKeybinding { .. } => None,
                BlockListMenuSource::RegularTextRightClick {
                    position_in_terminal_view,
                } => Some(*position_in_terminal_view),
                BlockListMenuSource::RichContentBlockRightClick {
                    position_in_terminal_view,
                    ..
                } => Some(*position_in_terminal_view),
                BlockListMenuSource::RichContentTextRightClick { .. } => None,
            },
            ContextMenuType::AltScreen { position } => Some(*position),
            ContextMenuType::Prompt { position } => Some(*position),
            ContextMenuType::Input { position } => Some(*position),
            ContextMenuType::AIBlockAttachedContext { .. } => None,
            ContextMenuType::AIBlockOverflowMenu { .. } => None,
            ContextMenuType::AgentViewEntryConversation { .. } => None,
        }
    }
}

#[derive(Copy, Clone)]
pub struct ContextMenuInfo {
    menu_type: ContextMenuType,
}

impl ContextMenuInfo {
    // This function should only be used for telemetry
    pub fn type_for_telemetry(&self) -> &'static str {
        match self.menu_type {
            ContextMenuType::BlockList { .. } => "Block",
            ContextMenuType::Prompt { .. } => "Prompt",
            ContextMenuType::Input { .. } => "Input",
            ContextMenuType::AltScreen { .. } => "AltScreen",
            ContextMenuType::AIBlockAttachedContext { .. } => "AIBlockContextList",
            ContextMenuType::AIBlockOverflowMenu { .. } => "AIBlockOverflowMenu",
            ContextMenuType::AgentViewEntryConversation { .. } => "AgentViewEntryConversation",
        }
    }

    // This function should only be used for telemetry
    pub fn open_method_for_telemetry(&self) -> &'static str {
        match self.menu_type {
            ContextMenuType::BlockList { menu_source } => match menu_source {
                BlockListMenuSource::BlockOverflowButton { .. } => "BlockOverflowButton",
                BlockListMenuSource::BlockKeybinding { .. } => "Keybinding",
                BlockListMenuSource::RegularBlockRightClick { .. } => "RightClick",
                BlockListMenuSource::RegularTextRightClick { .. } => "RightClick",
                BlockListMenuSource::RichContentBlockRightClick { .. } => "OutsideBlockRightClick",
                BlockListMenuSource::RichContentTextRightClick { .. } => "OutsideBlockRightClick",
                BlockListMenuSource::OutsideBlockRightClick { .. } => "OutsideBlockRightClick",
            },
            ContextMenuType::Prompt { .. } => "RightClick",
            ContextMenuType::Input { .. } => "RightClick",
            ContextMenuType::AltScreen { .. } => "AltScreen",
            ContextMenuType::AIBlockAttachedContext { .. } => "AIBlockAttachedBlockChipLeftClick",
            ContextMenuType::AIBlockOverflowMenu { .. } => "AIBlockOverflowMenuClick",
            ContextMenuType::AgentViewEntryConversation { .. } => "RightClick",
        }
    }
}

#[derive(Debug, Copy, Clone)]
struct ContextMenuState {
    menu_type: ContextMenuType,
}

#[derive(Copy, Clone)]
pub enum BlockEntity {
    Command,
    Output,
    FilteredOutput,
    CommandAndOutput,
}

impl BlockEntity {
    pub fn as_str(&self) -> &'static str {
        match self {
            BlockEntity::Command => "Command",
            BlockEntity::Output => "Output",
            BlockEntity::CommandAndOutput => "Both",
            BlockEntity::FilteredOutput => "FilteredOutput",
        }
    }
}

/// Represents the possible "states" of an items inclusion in blocklist AI context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AIContextInclusionState {
    /// The item will be included with the next AI query.
    Pending,

    /// The item was included as context in a past AI message in the active conversation.
    Active,
}

pub struct BlocklistAIRenderContext {
    /// The set of `BlockId`s corresponding to blocks to be included or previously included as AI
    /// context.
    ///
    /// This map is keyed by `ContextInclusionState`, where the corresponding set represents the
    /// blocks for that state.
    block_ids: HashMap<AIContextInclusionState, HashSet<BlockId>>,

    /// The ID of the selected Agent Mode conversation, if any.
    ///
    selected_conversation_id: Option<AIConversationId>,

    /// The IDs of exchanges in the selected conversation.
    exchange_ids: Option<HashSet<AIAgentExchangeId>>,

    /// `true` if we should highlight pending and active context in this conversation.
    pub should_highlight_context: bool,

    /// `true` if ai_input is enabled.
    pub is_ai_input_enabled: bool,

    /// `true` if there is pending context selected text attached.
    pub has_pending_context_selected_text: bool,
}

impl BlocklistAIRenderContext {
    /// Returns `true` if there's an active AI conversation.
    pub fn has_active_conversation(&self) -> bool {
        self.selected_conversation_id.is_some()
    }

    /// Returns `true` if the exchange with the given ID is in the active conversation.
    pub fn is_exchange_in_active_conversation(&self, id: &AIAgentExchangeId) -> bool {
        self.exchange_ids
            .as_ref()
            .is_some_and(|active_exchange_ids| active_exchange_ids.contains(id))
    }

    pub fn context_inclusion_state_for_block(
        &self,
        block: &Block,
    ) -> Option<AIContextInclusionState> {
        if let (Some(ai_metadata), Some(active_conversation_id)) = (
            block.agent_interaction_metadata(),
            self.selected_conversation_id.as_ref(),
        ) {
            if ai_metadata.conversation_id() == active_conversation_id {
                return Some(AIContextInclusionState::Active);
            }
        }

        [
            AIContextInclusionState::Pending,
            AIContextInclusionState::Active,
        ]
        .iter()
        .find(|state| {
            self.block_ids
                .get(state)
                .map(|ids| ids.contains(block.id()))
                .unwrap_or(false)
        })
        .copied()
    }

    /// Returns the AI context stripe color to use for a block, if any.
    pub fn context_color_for_block(&self, block: &Block, theme: &WarpTheme) -> Option<ColorU> {
        match self.context_inclusion_state_for_block(block) {
            Some(AIContextInclusionState::Active) => self.context_color(theme),
            _ => None,
        }
    }

    /// Returns the AI context stripe color to use for rich content, if any,
    pub fn context_color_for_rich_content(
        &self,
        rich_content: &RichContentMetadata,
        theme: &WarpTheme,
    ) -> Option<ColorU> {
        match rich_content {
            RichContentMetadata::AIBlock(ai_metadata)
                if self.is_exchange_in_active_conversation(&ai_metadata.exchange_id) =>
            {
                self.context_color(theme)
            }
            RichContentMetadata::AIOnboardingBlock { exchange_id, .. }
                if self.is_exchange_in_active_conversation(exchange_id) =>
            {
                self.context_color(theme)
            }
            _ => None,
        }
    }

    /// The context color to use for a block, given its conversation phase.
    /// This assumes the block is part of the active conversation.
    fn context_color(&self, theme: &WarpTheme) -> Option<ColorU> {
        (self.is_ai_input_enabled && self.should_highlight_context).then(|| ai_brand_color(theme))
    }
}

/// Groups together some structs to represent the state of the Terminal View for the
/// current frame. Passed to `AltScreenElement` and `BlockListElement`.
pub struct TerminalViewRenderContext {
    pub size_info: SizeInfo,
    pub scroll_position: ScrollPosition,
    pub highlighted_url: Option<GridHighlightedLink>,
    pub link_tool_tip: Option<GridHighlightedLink>,
    pub is_terminal_focused: bool,
    pub is_terminal_selecting: bool,
    pub is_context_menu_open: bool,
    pub is_waterfall_gap_mode: bool,
    pub pane_state: SplitPaneState,
    pub active_session_state: ActiveSessionState,
    pub selected_blocks: SelectedBlocks,
    /// Identifier for retrieving the position information of the input box element.
    pub input_box_element_key: String,
    /// Unique view id for saving active cursor position.
    pub terminal_view_id: EntityId,
    /// This map contains the IDs of sessions that were subshells as keys. Their corresponding
    /// values are the command that spawned the subshell, which is needed to paint the "flag"
    pub spawning_command_for_subshell_sessions: HashMap<SessionId, SubshellSource>,

    pub obfuscate_secrets: ObfuscateSecrets,
    pub hovered_secret: Option<SecretHandle>,

    pub horizontal_clipped_scroll_state: ClippedScrollStateHandle,

    /// Context for struct containing information about blocks and AI blocks used to render
    /// AI-specific decoration in the blocklist element.
    pub ai_render_context: Rc<RefCell<BlocklistAIRenderContext>>,
}

#[derive(Default)]
struct TerminalViewMouseStates {
    grid_link_tooltip: MouseStateHandle,
    rich_content_link_tooltip: MouseStateHandle,

    // Shared across Grid and Rich Content secrets tooltips (only 1 can be open at a time).
    toggle_secrets_tooltip: MouseStateHandle,
    copy_secrets_tooltip: MouseStateHandle,

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    open_in_warp_tooltip: MouseStateHandle,
    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    show_in_file_explorer_tooltip: MouseStateHandle,
    jump_to_bottom_of_block_button: MouseStateHandle,

    parent_conversation_header_link: MouseStateHandle,
    /// Persistent horizontal scroll state for the orchestration breadcrumb
    /// row. Lives here (rather than as a `MouseStateHandle`) so the user's
    /// scroll position survives across renders — in narrow split-off panes
    /// the breadcrumb row often overflows the title slot, and we wrap it
    /// in a `NewScrollable::horizontal` keyed on this handle so the user
    /// can pan to read clipped labels.
    breadcrumbs_horizontal_scroll: ClippedScrollStateHandle,
}

/// Where content was routed when sent to a CLI agent.
/// Returned by [`TerminalView::try_send_text_to_cli_agent_or_rich_input`]
/// so callers can report the correct telemetry destination without a
/// separate read of the rich input state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliAgentRouting {
    /// Content was inserted into CLI agent rich input.
    RichInput,
    /// Content was written directly to the PTY.
    Pty,
}

/// An enum representing the different states that a terminal view can be in,
/// based on any commands it's actively running and the result of the most
/// recent command that it finished.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum TerminalViewState {
    /// The most recent command had a non-successful exit code.
    Errored,
    /// Currently running a command.
    LongRunning,
    /// Not running any commands, and the last command it ran (if any) was
    /// successful.
    Normal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::terminal::view) enum ConversationDetailsPanelAutoOpenPolicy {
    #[default]
    DefaultOpen,
    DefaultClosed,
}

/// A struct containing information about a state change event for a particular
/// terminal view.
#[derive(Copy, Clone)]
pub struct TerminalViewStateChange {
    pub state: TerminalViewState,
    pub timestamp: Instant,
}

impl Default for TerminalViewStateChange {
    fn default() -> TerminalViewStateChange {
        TerminalViewStateChange {
            state: TerminalViewState::Normal,
            timestamp: Instant::now(),
        }
    }
}

/// Whether or not this is the active terminal session. The active session for a pane group
/// is the one used for executing workflows, Warp AI suggestions, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveSessionState {
    Active,
    Inactive,
}

enum SecretTooltip {
    Grid {
        is_agent_mode: bool,
        tooltip: WithinModel<SecretHandle>,
    },
    RichContent {
        is_agent_mode: bool,
        tooltip: RichContentSecretTooltipInfo,
    },
}

pub fn is_prompt_suggestions_enabled(app: &AppContext) -> bool {
    AISettings::as_ref(app).is_prompt_suggestions_enabled(app)
        && UserWorkspaces::as_ref(app).is_prompt_suggestions_toggleable()
}

type TerminalViewCallback = Box<dyn FnOnce(&mut TerminalView, &mut ViewContext<TerminalView>)>;
type ConversationFinishedCallback =
    Box<dyn FnOnce(&mut TerminalView, FinishReason, &mut ViewContext<TerminalView>)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::terminal::view) enum PendingUserQueryKind {
    QueuedPrompt,
    CloudMode,
}

#[derive(Debug, Clone)]
pub struct TerminalDropTargetData {
    pub terminal_view: WeakViewHandle<TerminalView>,
}

impl DropTargetData for TerminalDropTargetData {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub struct TerminalView {
    pub model: Arc<FairMutex<TerminalModel>>,
    view_handle: WeakViewHandle<Self>,

    /// The session's size data. This is wrapped in a [`Tracked`] to
    /// guarantee that the [`TerminalView`] is redrawn whenever the
    /// size info changes.
    size_info: Tracked<SizeInfo>,

    /// The input area at the bottom of the viewport.
    input: ViewHandle<Input>,

    inline_menu_positioner: ModelHandle<InlineMenuPositioner>,

    /// Colors used for rendering.
    colors: color::List,

    /// The current scroll position.
    scroll_position: ScrollState,

    /// Cached scroll position from before entering agent view, used to restore on exit.
    scroll_position_before_entering_agent_view: Option<ScrollPosition>,

    /// Scroll state for scrolling vertically in the blocklist.
    blocklist_vertical_scroll_state: ScrollStateHandle,

    /// Scroll state for scrolling vertically in the alt screen.
    /// This only happens if we're a shared session viewer and
    /// our window is smaller than the sharer's.
    alt_screen_vertical_scroll_state: ScrollStateHandle,
    /// Lines from the top of the content we are scrolled in the alt screen.
    alt_screen_scroll_top: Lines,

    /// Scroll state for scrolling horizontally.
    horizontal_clipped_scroll_state: ClippedScrollStateHandle,

    /// Whether there is an active text selection.
    is_selecting: bool,

    context_menu: ViewHandle<Menu<TerminalAction>>,

    /// None iff there is no context menu open currently.
    context_menu_state: Option<ContextMenuState>,

    /// The search bar at the top of the terminal view.
    find_bar: ViewHandle<Find<TerminalFindModel>>,

    /// The block whose filter we are actively editing.
    active_filter_editor_block_index: Option<BlockIndex>,
    block_filter_editor: ViewHandle<BlockFilterEditor>,

    hovered_block_index: Option<BlockIndex>,

    selected_blocks: SelectedBlocks,

    // Whether any session contains blocks from a remote session. Cached to improve performance.
    // Blocks don't necessarily need to be finished for this to be true (e.g. it's true for
    // an empty ssh session where just the active block is remote).
    any_session_contains_remote_blocks: bool,

    // Whether any session contains restored blocks from a remote session. Cached to improve performance.
    any_session_contains_restored_remote_blocks: bool,

    /// Mouse state for our block list element.
    block_list_mouse_states: BlockListMouseStates,

    /// All state related to the floating command header ("the snackbar")
    snackbar_header_state: SnackbarHeaderState,

    /// The block index of the block the user has moused down on. This is a
    /// temporary state to determine if a single block has been clicked.
    mouse_down_block_index: Option<BlockIndex>,

    mouse_states: TerminalViewMouseStates,

    server_api: Arc<ServerApi>,
    auth_state: Arc<AuthState>,

    /// A sender used to handle messages for whenever the entire terminal view
    /// changes size.  Note that this size contains not just the content element
    /// but also the input.
    resize_tx: Sender<Vector2F>,

    find_link_tx: Sender<FindLinkArg>,

    /// Highlighted link (could be url or file path) on the screen.
    highlighted_link: HighlightedLinkOption,
    open_grid_link_tool_tip: Option<GridHighlightedLink>,

    open_rich_content_link_tool_tip: Option<RichContentLinkTooltipInfo>,

    last_hover_fragment_boundary: Option<WithinModel<FragmentBoundary>>,

    bootstrap_start: Option<Instant>,
    is_login_shell_bootstrapped: bool,
    /// Set when a pending command is submitted to the shell. Cleared on the
    /// next `AfterBlockCompleted`, at which point `Event::PendingCommandCompleted`
    /// is emitted so subscribers know the command has finished.
    awaiting_pending_command_completion: bool,
    /// Commands that should run as separate blocks after the active pending
    /// command finishes successfully.
    pending_command_queue: VecDeque<String>,
    /// When true, enter agent view after pending setup commands complete
    /// (i.e. after `PendingCommandCompleted` is emitted). Set by
    /// `pane_tree_from_template_recursive` when a tab config has both
    /// commands and `PaneMode::Agent`.
    enter_agent_view_after_pending_commands: bool,
    slow_bootstrap_banner: ViewHandle<Banner<TerminalAction>>,
    is_slow_bootstrap_banner_open: bool,

    /// The handle to any currently hovered secret. Used to determine whether the
    /// secret gets a special hovered treatment.
    hovered_secret: Option<SecretHandle>,

    /// The details of a currently focused secret tooltip (either grid or rich content).
    open_secret_tool_tip: Option<SecretTooltip>,

    control_master_error_banner: ViewHandle<Banner<TerminalAction>>,
    control_master_error_banner_state: ControlMasterErrorBannerState,

    /// Banner to show if we detect a configuration in the user's rc files that
    /// is incompatible with Warp.
    incompatible_configuration_banner: ViewHandle<Banner<TerminalAction>>,
    is_incompatible_configuration_banner_open: bool,

    /// Non-MacOS banner to ask if the user prefers MacOS bindings
    /// or Emacs-style bindings for `ctrl-a` and `ctrl-e`.
    emacs_bindings_banner: ViewHandle<Banner<TerminalAction>>,
    is_emacs_bindings_banner_open: bool,

    pane_configuration: ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,

    sessions: ModelHandle<Sessions>,
    active_block_metadata: Option<BlockMetadata>,

    block_text_selection_start_position: Option<Vector2F>,

    /// Background executor for sending telemetry when a TerminalView is
    /// dropped.
    background_executor: Arc<Background>,

    inline_banners_state: InlineBannersState,

    /// Most recent command correction encountered, if any, used for the keyboard shortcut action.
    most_recent_command_correction: Option<Correction>,

    /// Set of block indexes that are bookmarked, including the mouse states for their indicators
    bookmarked_blocks: HashMap<BlockIndex, MouseStateHandle>,

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    file_link_scanning_join_handle: Option<JoinHandle<()>>,

    last_focus_ts: Option<NaiveDateTime>,
    tips_completed: ModelHandle<TipsCompleted>,

    /// A manually managed [`PrivacySettingsSnapshot`]. We must maintain a separate snapshot of
    /// [`PrivacySettings`] (rather than using it directly), so we can decide whether to send a
    /// telemetry event in the view's `drop()` method, which does not have access to a ViewContext
    /// (which is required for reading the `PrivacySettings` model). This is a less-than-ideal
    /// workaround; other usages of PrivacyModel should directly read from the singleton model
    /// managed by the UI framework (e.g. via `PrivacySettings::handle(ctx)`).
    privacy_settings_snapshot: PrivacySettingsSnapshot,

    /// Whether or not this terminal session was ever active.
    was_ever_visible: bool,

    /// The [`EntityId`] for this terminal view.
    view_id: EntityId,

    current_state: TerminalViewStateChange,

    /// Whether we've already emitted a chrome refresh for the active block after it crossed the
    /// long-running threshold. Reset when the active command starts and finishes.
    did_notify_long_running: bool,

    /// This field is an "&&" combination of two other pieces of state:
    ///   1. Whether this View (or one of its children) is the focused View.
    ///   2. Whether this View's window is the active window.
    ///
    /// We need to derive and cache this state on this View in order to correctly implement focus
    /// reporting. Because focus is window-scoped, i.e. warpui does not consider activating a
    /// different window as blurring the focused View in the previously active window, we cannot
    /// simply rely on the warpui::View::on_blur and on_focus methods to report focus-in/out to the
    /// PTY, as those methods will not trigger when changing active windows. The singleton model
    /// [`warpui::windowing::State`] will allow us to subscribe to active window change. So, we can
    /// subscribe to that and have that callback also report focus-in/out. However, that will still
    /// leave cases for potential double-reporting, as a single click can trigger both
    /// [`warpui::View::on_focus`] and emit a [`warpui::windowing::StateEvent`]. This field will
    /// guard against that double- reporting case, though it needs to be kept in sync with the
    /// focused view and active window.
    is_focused_and_active: bool,

    current_prompt: ModelHandle<PromptType>,

    model_event_sender: Option<SyncSender<persistence::ModelEvent>>,

    /// The child views that represent rich content. These can be inserted into the block list with
    /// the `insert_rich_content` helper function.
    rich_content_views: Vec<RichContent>,
    pending_user_query_view_id: Option<EntityId>,
    pending_user_query_kind: Option<PendingUserQueryKind>,
    queued_prompt_callback: Option<ConversationFinishedCallback>,

    /// Cached view ids for usage footers keyed by the AI block view id that owns them.
    usage_footer_view_ids: HashMap<EntityId, EntityId>,

    // Whether the block onboarding view is active or not.
    block_onboarding_active: bool,

    // View handles for the onboarding blocks.
    onboarding_prompt_block: Option<ViewHandle<OnboardingPromptBlock>>,
    settings_import_onboarding_block: Option<ViewHandle<SettingsImportView>>,
    onboarding_agentic_suggestions_block: Option<ViewHandle<OnboardingAgenticSuggestionsBlock>>,

    onboarding_callout_view: Option<ViewHandle<onboarding::OnboardingCalloutView>>,

    // If the agentic suggestions onboarding block is pending, mark it here.
    pending_onboarding_agentic_suggestions_block: bool,

    /// The type of the subshell that we will bootstrap/"warpify"" on the next [`AfterBlockStarted`]
    /// terminal model event. Will only be `Some` with a [`ShellType`] we can bootstrap.
    pending_auto_bootstrap_shell_type: Option<ShellType>,
    env_vars: Vec<EnvVar>,

    show_snackbar: bool,
    hover_near_snackbar_area: bool,

    ai_controller: ModelHandle<BlocklistAIController>,
    passive_suggestions_models: PassiveSuggestionsModels,
    ai_action_model: ModelHandle<BlocklistAIActionModel>,
    ai_input_model: ModelHandle<BlocklistAIInputModel>,
    ai_context_model: ModelHandle<BlocklistAIContextModel>,
    get_relevant_files_controller: ModelHandle<GetRelevantFilesController>,

    pending_env_var_collection: Option<CloudEnvVarCollection>,

    ai_render_context: Rc<RefCell<BlocklistAIRenderContext>>,

    // TODO(suraj): consider flattening this to the [`SharedSessionKind`]
    // and adding a `Unshared` variant to it. This would require [`SharedSessionKind::Sharer`]
    // and [`SharedSessionKind::Viewer`] to store some common struct for common fields.
    shared_session: Option<SharedSessionAdapter>,

    /// Stashed source from `attempt_to_share_session` so `on_session_share_started`
    /// can decide whether to auto-copy the link vs open the sharing dialog.
    pending_share_source: Option<SharedSessionActionSource>,

    /// When true, automatically stop the shared session when the CLI agent session ends.
    /// Set when sharing is started from the remote control entrypoint.
    auto_stop_sharing_on_cli_end: bool,

    /// The inserted conversation-ended tombstone, if this view currently has one.
    conversation_ended_tombstone_view_id: Option<EntityId>,

    /// The ID of the containing window.
    window_id: WindowId,

    /// The position ID of the currently rendered terminal "content" element; either the blocklist
    /// element or the alt screen element depending on which is currently rendered.
    content_element_position_id: String,

    /// The position ID of the terminal input.
    ///
    /// This is cached, as opposed to read from `Input` on demand, to prevent otherwise-possible
    /// circular view references that could occur because `TerminalView` implements the `MenuPositioningProvider`
    /// that's used as a dependency of certain `Input` methods. `MenuPositioningProvider`
    /// internally relies on reading the last-frame position of `Input`, which would otherwise
    /// require reading the position ID directly from `Input` and cause a circular ref panic.
    input_position_id: String,

    /// A handle for the [`Hoverable`] that we render the [`Input`] view in.
    ///
    /// While the [`Input`] itself might internally render with a [`Hoverable`]
    /// around it, we use a dedicated [`Hoverable`] at the [`TerminalView`] level because
    /// 1. the [`Input`] implementation might change, and
    /// 2. we have specific hover behaviour at the [`TerminalView`] level
    ///    (e.g. a hover-out delay)
    input_hoverable_handle: MouseStateHandle,

    find_model: ModelHandle<TerminalFindModel>,

    warpify_state: WarpifyState,

    /// The keystroke bound to canceling a command.
    ///
    /// This is cached on the view because the UI framework APIs needed to lookup keystroke for an
    /// action only exist on `AppContext`, which is not accessible at render time. Sigh.
    cancel_command_keystroke: Option<Keystroke>,

    /// Whether the terminal view is currently a drop target for a file. If it is, we render an overlay.
    is_file_drop_target: bool,

    /// Whether this terminal pane is taking care of uploading a file over SSH.
    is_ssh_file_uploader: bool,

    /// The file uploads initiated in this terminal pane.
    ssh_file_upload: ViewHandle<FileUpload>,

    /// The type of the shell that this terminal pane is running, derived and
    /// cached on the view from [`ShellLaunchdata`]. Used to render an indicator
    /// in the tab bar.
    shell_indicator_type: Option<ShellIndicatorType>,

    /// Used to describe the active shell to the user.
    shell_detail: Option<String>,

    /// Position ID for this view.
    position_id: String,

    /// Position ID for the active terminal cursor.
    cursor_position_id: String,

    #[cfg_attr(not(test), allow(unused))]
    active_session: ModelHandle<ActiveSession>,

    pty_spawn_failed: bool,

    model_events_handle: ModelHandle<ModelEventDispatcher>,

    is_todo_popup_visible: bool,

    agent_todos_popup: ViewHandle<AgentTodosPopupView>,

    /// Per-repo git status model for the current repository, if any.
    #[cfg(feature = "local_fs")]
    git_repo_status: Option<ModelHandle<GitRepoStatusModel>>,

    /// Deferred code review open request, stashed when [`GitDeltaPreference::OnlyDirty`] is
    /// requested but git status metadata has not loaded yet. Consumed in
    /// [`Self::handle_git_repo_status_event`].
    #[cfg(feature = "local_fs")]
    deferred_code_review_open: Option<DeferredCodeReviewOpen>,

    /// A list of callbacks to run on the next [`ModelEvent::AfterBlockCompleted`] received.
    block_completed_callbacks: Vec<TerminalViewCallback>,

    /// A list of callbacks to run on the next
    /// [`BlocklistAIControllerEvent::FinishedReceivingOutput`] received, regardless of the finish reason.
    conversation_completed_callbacks: Vec<ConversationFinishedCallback>,

    /// Path to the current repository, or None if not currently in a repo.
    current_repo_path: Option<LocalOrRemotePath>,

    /// The title of the terminal view to show when there is no selected conversation.
    terminal_title: String,

    // If there is a selected conversation in the view before bootstrapping (from loading a conversation into a new pane),
    // we want to keep the title as the conversation title, so we should ignore the model event setting the title after bootstrapping finishes
    ignore_next_set_title_event: bool,

    cli_subagent_views: HashMap<BlockId, ViewHandle<CLISubagentView>>,
    cli_subagent_controller: ModelHandle<CLISubagentController>,
    use_agent_footer: ViewHandle<UseAgentToolbar>,

    agent_view_controller: ModelHandle<AgentViewController>,
    agent_view_back_button: ViewHandle<ActionButton>,
    /// Pill bar shown above the agent view header listing the orchestrator and
    /// child agents. Gated by `FeatureFlag::OrchestrationPillBar`. The view is
    /// always constructed; render-time guards control whether it draws anything.
    orchestration_pill_bar: ViewHandle<OrchestrationPillBar>,
    /// `true` when this view hosts a child agent split off into its own
    /// pane/tab. Drives breadcrumb-vs-pill-bar rendering in the pane header.
    is_orchestration_split_off: bool,
    is_using_conversation_for_pane_header_title: bool,

    ambient_agent_view_model: Option<ModelHandle<ambient_agent::AmbientAgentViewModel>>,
    pending_cloud_followup_task_id: Option<AmbientAgentTaskId>,

    /// Conversation details panel (side panel showing conversation/task metadata).
    /// Available for cloud Oz runs and for any active local AI conversation.
    conversation_details_panel:
        ViewHandle<crate::ai::conversation_details_panel::ConversationDetailsPanel>,
    /// Whether the conversation details panel is currently open.
    is_conversation_details_panel_open: bool,
    /// Whether we've already auto-opened the panel when the agent started running.
    /// This prevents re-opening the panel if the user manually closes it. Only set
    /// by the cloud-mode auto-open path; local conversations require the user to
    /// click the pane-header toggle button to open the panel.
    has_auto_opened_conversation_details_panel: bool,
    /// Determines whether the one-shot auto-open should open the panel or be
    /// consumed without opening.
    conversation_details_panel_auto_open_policy: ConversationDetailsPanelAutoOpenPolicy,
    /// Mouse state handle for the conversation details panel toggle button in the pane header.
    /// Only available on non-WASM platforms (WASM uses a per-window button instead).
    #[cfg(not(target_arch = "wasm32"))]
    conversation_details_panel_toggle_mouse_state: warpui::elements::MouseStateHandle,
    /// Mouse state handle for the ambient agent cancel button in the pane header.
    ambient_agent_cancel_mouse_state: warpui::elements::MouseStateHandle,

    /// First-time cloud agent setup view (full-screen overlay for creating initial environment).
    first_time_cloud_agent_setup_view: ViewHandle<ambient_agent::FirstTimeCloudAgentSetupView>,

    /// Environment setup mode selector modal for /create-environment command.
    environment_setup_mode_selector: ViewHandle<EnvironmentSetupModeSelector>,

    /// Whether the environment setup mode selector is currently visible.
    is_environment_setup_mode_selector_open: bool,

    /// Weak handle to the [`PaneStack`] this view is part of, allowing push/pop operations.
    pane_stack: Option<WeakModelHandle<crate::pane_group::pane::PaneStack<Self>>>,

    /// If set, indicates a cloud mode entry is waiting for the fullscreen agent view to be exited.
    /// This is used to ensure rich content inserted for cloud mode is scoped to the top-level
    /// terminal view (not a specific agent view conversation).
    pending_cloud_mode_start_callback: Option<TerminalViewCallback>,
    pending_cloud_mode_start_abort_handle: Option<SpawnedFutureHandle>,

    /// Active /init flow model, if any. Cleared when cancelled or completed.
    active_init_project_model: Option<ModelHandle<InitProjectModel>>,

    /// Whether we're waiting for the result of an AWS CLI login command.
    /// Used to detect "command not found" errors when AWS CLI isn't installed.
    /// TODO: In the future, when we support GCP/Azure cloud CLIs, this should be
    /// converted to `pending_cloud_cli_login: Option<CloudProvider>` where CloudProvider
    /// is an enum with variants like Aws, Gcp, Azure.
    is_pending_aws_login: bool,
    /// `true` if this view explicitly requested a PTY shutdown.
    ///
    /// Once set, this remains true for the rest of the view's lifecycle and
    /// suppresses `AgentExitedShellProcess` telemetry so manual shutdown paths
    /// (tab close, update relaunch, etc.) are not attributed to agent commands.
    manual_pty_shutdown_requested: bool,

    ephemeral_message_model: ModelHandle<EphemeralMessageModel>,

    /// Per-session PTY recorder for writing PTY bytes to a file.
    pty_recorder: ModelHandle<PtyRecorder>,

    /// When viewer-driven sizing is active on the sharer, this stores the
    /// viewer's last reported (rows, cols).
    /// Used by `SizeUpdateBuilder::build()` to prevent `AfterLayout` from
    /// overriding the viewer-reported size back to the sharer's natural pane size.
    active_viewer_driven_size: Option<(usize, usize)>,

    /// State handle for the shimmering text animation in the remote server loading footer.
    /// Persisted across renders so the animation doesn't restart.
    remote_server_shimmer_handle: ShimmeringTextStateHandle,
}

/// Parameters stashed when a code review pane open is requested with
/// [`GitDeltaPreference::OnlyDirty`] but git status metadata is not yet available.
/// Consumed once the per-repo [`GitRepoStatusModel`] delivers its first update.
#[cfg(feature = "local_fs")]
struct DeferredCodeReviewOpen {
    git_delta_preference: GitDeltaPreference,
    focus_new_pane: bool,
}

#[derive(Copy, Clone, Serialize)]
pub enum BlockSelectionDelta {
    // User first selects a block, or selects a block with click
    New,
    // User already has block selected, and selects previous block
    Previous,
    // User already has block selected, and selects next block
    Next,
}

#[derive(Copy, Clone, Serialize)]
pub struct BlockSelectionDetails {
    cardinality: BlockSelectionCardinality,
    delta: BlockSelectionDelta,
    is_cmd_down: bool,
    is_shift_down: bool,
}

/// Why `apply_block_metadata_update` is being invoked. The two sources have
/// different cardinalities — precmd fires exactly once per block, whereas OSC 7
/// can fire many times mid-block from chatty prompts. Once-per-block work
/// (git-repo detection on unchanged CWDs, `block_completed_callbacks` drain)
/// must be gated on this distinction.
#[derive(Copy, Clone, Debug)]
enum BlockMetadataUpdateSource {
    /// `Event::BlockMetadataReceived` — the shell's precmd hook fired between
    /// blocks. Run all once-per-block work.
    Precmd,
    /// `Event::BlockWorkingDirectoryUpdated` — the running command emitted an
    /// OSC 7 sequence (`\e]7;file://host/path\a`). Skip repo detection unless
    /// the CWD actually changed, and never run block-completion callbacks
    /// (the block hasn't completed).
    Osc7,
}

/// Constructs the keybindings struct for the onboarding callout.
///
/// Gets display strings for:
/// - Toggle input mode: from TerminalKeybindings (editable binding)
/// - Submit to local agent: fixed binding (cmd-enter / ctrl-shift-enter)
/// - Submit to cloud agent: fixed binding (cmd-alt-enter / ctrl-alt-enter)
fn build_onboarding_keybindings(ctx: &AppContext) -> OnboardingKeybindings {
    let toggle_input_mode = TerminalKeybindings::handle(ctx)
        .as_ref(ctx)
        .set_input_mode_agent_keybinding()
        .unwrap_or_else(|| {
            if OperatingSystem::get().is_mac() {
                "⌘-I".to_string()
            } else {
                "Ctrl-I".to_string()
            }
        });

    // EditorAction::CmdEnter is a fixed binding, not editable
    let submit_to_local_agent = if OperatingSystem::get().is_mac() {
        Keystroke::parse("cmd-enter")
    } else {
        Keystroke::parse("ctrl-shift-enter")
    }
    .map(|k| k.displayed())
    .unwrap_or_else(|_| "⌘-⏎".to_string());

    // TerminalAction::EnterCloudAgentView is a fixed binding, not editable
    let submit_to_cloud_agent = if OperatingSystem::get().is_mac() {
        Keystroke::parse("cmd-alt-enter")
    } else {
        Keystroke::parse("ctrl-alt-enter")
    }
    .map(|k| k.displayed())
    .unwrap_or_else(|_| "⌘-⌥-⏎".to_string());

    let return_to_terminal_mode = Keystroke::parse("escape")
        .map(|k| k.displayed())
        .unwrap_or_else(|_| "ESC".to_string());

    OnboardingKeybindings {
        toggle_input_mode,
        submit_to_local_agent,
        submit_to_cloud_agent,
        return_to_terminal_mode,
    }
}

/// Builds the context-menu label for forking an AI conversation from a given query.
fn fork_label_for_query(query: &str) -> String {
    if query.is_empty() {
        "Fork from last query".to_string()
    } else {
        let first_line = query.lines().next().unwrap_or(query).trim();
        let chars: Vec<char> = first_line.chars().take(21).collect();
        let (truncated, suffix) = if chars.len() > 20 {
            (chars[..20].iter().collect::<String>(), "…")
        } else {
            (chars.iter().collect::<String>(), "")
        };
        format!("Fork from \"{truncated}{suffix}\"")
    }
}

impl TerminalView {
    /// Returns the path to the current repository, if any.
    pub fn current_repo_path(&self) -> Option<&LocalOrRemotePath> {
        self.current_repo_path.as_ref()
    }

    /// Returns the local repo path, if the current repo is local.
    /// Remote repo paths return None — full remote support is a follow-up.
    pub fn current_local_repo_path(&self) -> Option<&Path> {
        self.current_repo_path
            .as_ref()
            .and_then(|p| p.to_local_path())
    }

    fn is_nested_cloud_mode(&self, app: &AppContext) -> bool {
        if !self.is_ambient_agent_session(app) {
            return false;
        }

        let Some(pane_stack) = self
            .pane_stack
            .as_ref()
            .and_then(|handle| handle.upgrade(app))
        else {
            return false;
        };

        pane_stack
            .as_ref(app)
            .entries()
            .iter()
            .position(|(_, view)| view.id() == self.view_id)
            .is_some_and(|index| index > 0)
    }

    /// Create a SyncEvent for other terminals to use based on
    /// the state of this terminal. If this terminal view has an active input
    /// editor, other terminals should match those contents.
    /// Otherwise, they should just start syncing.
    pub fn create_sync_event_based_on_terminal_state(&self, app_ctx: &AppContext) -> SyncEvent {
        if !matches!(
            self.model.lock().terminal_input_state(),
            TerminalInputState::InputEditor,
        ) {
            return SyncEvent {
                source_view_id: self.view_id,
                data: SyncInputType::StartSyncing,
            };
        }

        let input_buffer = self.input().as_ref(app_ctx).buffer_text(app_ctx);

        SyncEvent {
            source_view_id: self.view_id,
            data: SyncInputType::InputEditorContentsChanged {
                contents: Arc::new(input_buffer),
            },
        }
    }

    /// Marks rich content views as dirty if their metadata matches the given predicate.
    ///
    /// Rich content heights are stored in the blocklist sumtree. When a view's rendered height
    /// changes (e.g., due to state changes that affect its layout), the sumtree entry becomes
    /// stale. Marking items as dirty ensures they are re-measured on the next layout frame,
    /// which happens unconditionally before viewport iteration. This is important for items
    /// that may have 0 height in the sumtree, as the viewport iterator would otherwise skip
    /// them entirely.
    fn mark_all_rich_content_items_dirty_where(
        &self,
        model: &mut TerminalModel,
        predicate: impl Fn(&RichContentMetadata) -> bool,
    ) {
        for content in &self.rich_content_views {
            if content.metadata().is_some_and(&predicate) {
                model
                    .block_list_mut()
                    .mark_rich_content_dirty(content.view_id());
            }
        }
    }

    /// Receives a SyncEvent and performs actions dictated by that event
    /// on this terminal view.
    pub fn receive_sync_input_event(&mut self, event: &SyncEvent, ctx: &mut ViewContext<Self>) {
        // The source terminal shouldn't process it's own data sync event.
        if event.source_view_id == self.view_id {
            return;
        }

        let terminal_input_state = self.model.lock().terminal_input_state();

        match &event.data {
            SyncInputType::InputEditorContentsChanged { contents } => {
                if matches!(
                    terminal_input_state,
                    TerminalInputState::InputEditor | TerminalInputState::NotBootstrapped
                ) {
                    self.input.update(ctx, |input, ctx| {
                        input.send_input_buffer_to_terminal_editor(Arc::clone(contents), ctx);
                    })
                }
            }
            SyncInputType::NonEditorTyped { chars: typed_chars } => {
                if matches!(
                    terminal_input_state,
                    TerminalInputState::LongRunningCommand | TerminalInputState::AltScreen,
                ) {
                    self.write_to_pty_for_syncing_long_running_commands(typed_chars.to_vec(), ctx);
                }
            }
            SyncInputType::RanCommand => {
                if terminal_input_state == TerminalInputState::InputEditor {
                    self.input.update(ctx, |input, ctx| {
                        input.run_command_in_synced_terminal_input(ctx);
                    });
                }
            }
            // For start and stop syncing we only need to change the input box
            // show/hide logic and that's handled before this match statement.
            SyncInputType::StartSyncing => (),
            SyncInputType::StopSyncing => {}
        }
    }

    /// Returns whether local input-editor CRDT edits should be published to the shared-session
    /// sharer. Viewer-local editor events can still fire from ended/setup-only cloud agent surfaces,
    /// where sending them upstream would be rejected and surfaced back as edit failures.
    pub(crate) fn should_publish_shared_session_input_editor_update(
        &self,
        model: &TerminalModel,
        app: &AppContext,
    ) -> bool {
        let input_is_visible = self.is_input_box_visible(model, app);
        // If there is a conversation tombstone and the input is hidden, should not broadcast input updates as
        // the cloud agent session is over.
        self.conversation_ended_tombstone_view_id.is_none() || input_is_visible
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        resources: TerminalViewResources,
        wakeups_rx: Receiver<()>,
        model_events_handle: ModelHandle<ModelEventDispatcher>,
        model: Arc<FairMutex<TerminalModel>>,
        sessions: ModelHandle<Sessions>,
        size_info: SizeInfo,
        colors: List,
        model_event_sender: Option<SyncSender<persistence::ModelEvent>>,
        current_prompt: ModelHandle<PromptType>,
        initial_input_config: Option<InputConfig>,
        conversation_restoration: Option<ConversationRestorationInNewPaneType>,
        inactive_pty_reads_rx: Option<async_broadcast::InactiveReceiver<Arc<Vec<u8>>>>,
        is_cloud_mode: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let terminal_view_id = ctx.view_id();
        let active_session = ctx.add_model(|ctx| {
            ActiveSession::new(sessions.clone(), model_events_handle.clone(), ctx)
        });
        let ambient_agent_view_model = is_cloud_mode.then(|| {
            ctx.add_model(|ctx| ambient_agent::AmbientAgentViewModel::new(terminal_view_id, ctx))
        });

        let ephemeral_message_model = ctx.add_model(|_| EphemeralMessageModel::new());

        let agent_view_controller = ctx.add_model(|_| {
            AgentViewController::new(
                model.clone(),
                terminal_view_id,
                ephemeral_message_model.clone(),
            )
        });

        ctx.subscribe_to_model(&agent_view_controller, |me, _, event, ctx| {
            match event {
                AgentViewControllerEvent::EnteredAgentView {
                    display_mode,
                    conversation_id,
                    is_new,
                    origin,
                    ..
                } => {
                    // Clear prompt suggestions shown in the context of the terminal mode or prior agent view.
                    me.clear_prompt_suggestions(ctx);
                    match display_mode {
                        AgentViewDisplayMode::Inline => {
                            // Insert the inline agent view header as rich content
                            let header_view = ctx.add_view(|ctx| {
                                InlineAgentViewHeader::new(
                                    me.view_id,
                                    me.model.clone(),
                                    me.sessions.clone(),
                                    me.ai_action_model.clone(),
                                    ctx,
                                )
                            });
                            me.insert_rich_content(
                                Some(RichContentType::InlineAgentViewHeader),
                                header_view,
                                Some(RichContentMetadata::InlineAgentViewHeader),
                                RichContentInsertionPosition::Append {
                                    insert_below_long_running_block: false,
                                },
                                ctx,
                            );
                        }
                        AgentViewDisplayMode::FullScreen => {
                            let has_pending_blocks = !me
                                .ai_context_model
                                .as_ref(ctx)
                                .pending_context_block_ids()
                                .is_empty();
                            let should_insert_zero_state_block = *is_new
                                && !has_pending_blocks
                                && !matches!(
                                    origin,
                                    AgentViewEntryOrigin::CreateEnvironment
                                        | AgentViewEntryOrigin::SlashInit
                                        | AgentViewEntryOrigin::ThirdPartyCloudAgent
                                );
                            if should_insert_zero_state_block {
                                let mut should_show_init_callout = false;
                                if let Some(directory) = me.current_local_repo_path() {
                                    should_show_init_callout = me
                                        .should_show_agent_mode_setup_for_directory(directory, ctx);
                                    if should_show_init_callout {
                                        me.mark_agent_init_callout_as_shown_for_directory(
                                            directory, ctx,
                                        );
                                    }
                                }
                                let agent_view_zero_state = ctx.add_typed_action_view(|ctx| {
                                    AgentViewZeroStateBlock::new(
                                        *conversation_id,
                                        *origin,
                                        me.agent_view_controller.clone(),
                                        &me.sessions,
                                        me.ambient_agent_view_model.as_ref(),
                                        me.model.clone(),
                                        &me.model_events_handle,
                                        should_show_init_callout,
                                        ctx,
                                    )
                                });
                                ctx.subscribe_to_view(
                                    &agent_view_zero_state,
                                    |me, _, event, ctx| match event {
                                        AgentViewZeroStateEvent::ClickedInitCallout => {
                                            me.input.update(ctx, |input, ctx| {
                                                input.replace_buffer_content(
                                                    commands::INIT.name,
                                                    ctx,
                                                );
                                            });
                                        }
                                        AgentViewZeroStateEvent::OpenConversation {
                                            conversation_id,
                                        } => {
                                            me.enter_agent_view_for_conversation(
                                                None,
                                                AgentViewEntryOrigin::ConversationListView,
                                                *conversation_id,
                                                ctx,
                                            );
                                        }
                                    },
                                );
                                me.insert_rich_content(
                                    Some(RichContentType::AgentViewZeroState),
                                    agent_view_zero_state,
                                    Some(RichContentMetadata::AgentViewZeroState),
                                    RichContentInsertionPosition::Append {
                                        insert_below_long_running_block: false,
                                    },
                                    ctx,
                                );
                            }

                            // On agent-view-enter, we want to scroll to the bottom of the view
                            // (and save the current scroll position so we can get back to it when we exit the agent view).
                            me.scroll_position_before_entering_agent_view =
                                Some(me.scroll_position.position());
                            me.update_scroll_position_locking(
                                ScrollPositionUpdate::AfterEnterAgentView,
                                ctx,
                            );
                            me.update_agent_view_back_button_state(ctx);
                            ctx.notify();
                        }
                    }
                }
                AgentViewControllerEvent::ExitedAgentView {
                    conversation_id,
                    origin,
                    original_exchange_count,
                    final_exchange_count,
                    was_ambient_agent,
                    is_exit_before_new_entrance,
                    ..
                } => {
                    // Prompt suggestions should not follow the user back to terminal view.
                    me.clear_prompt_suggestions(ctx);
                    // For ambient agent sessions, pop the pane stack to return to the parent terminal.
                    // Skip the pop when this exit is immediately followed by re-entering agent view
                    // for a different conversation (e.g. a restored conversation taking over the
                    // pane).
                    if *was_ambient_agent && !*is_exit_before_new_entrance {
                        if let Some(pane_stack) =
                            me.pane_stack.as_ref().and_then(|h| h.upgrade(ctx))
                        {
                            pane_stack.update(ctx, |stack, ctx| {
                                stack.pop(ctx);
                            });
                        }
                    }

                    // Clean up any rich content scoped to the agent view 'lifetime'.
                    let view_ids_to_remove = me
                        .rich_content_views
                        .iter()
                        .filter_map(|view| {
                            let is_lrc_header =
                                matches!(origin, AgentViewEntryOrigin::LongRunningCommand)
                                    && view.is_inline_agent_view_header();
                            let is_agent_view_zero_state = view.is_agent_view_zero_state();
                            (is_lrc_header || is_agent_view_zero_state).then_some(view.view_id())
                        })
                        .collect_vec();
                    for view_id_to_remove in view_ids_to_remove.into_iter() {
                        me.model
                            .lock()
                            .block_list_mut()
                            .remove_rich_content(view_id_to_remove);
                        me.rich_content_views
                            .retain(|view| view.view_id() != view_id_to_remove);
                        ctx.notify();
                    }

                    // On exit-agent-view, we go back to the scroll position that we had when we entered.
                    if let Some(saved_position) =
                        me.scroll_position_before_entering_agent_view.take()
                    {
                        me.update_scroll_position_locking(
                            ScrollPositionUpdate::AfterExitAgentView { saved_position },
                            ctx,
                        );
                    }

                    let has_init_steps = me.has_init_steps_for_conversation(*conversation_id);

                    // Exiting agent view should cancel setup flows that hide the input box.
                    if me.has_active_init_project(ctx) && has_init_steps {
                        if let Some(model) = me.active_init_project_model.clone() {
                            model.update(ctx, |model, ctx| model.cancel(ctx));
                        }
                    }
                    for rich_content in me.rich_content_views.iter().rev() {
                        if rich_content.agent_view_conversation_id() != Some(*conversation_id) {
                            continue;
                        }

                        match rich_content.metadata() {
                            Some(RichContentMetadata::InitEnvironment { block_handle })
                                if !block_handle.as_ref(ctx).completed() =>
                            {
                                block_handle.update(ctx, |block, ctx| block.handle_ctrl_c(ctx));
                            }
                            Some(RichContentMetadata::EnvVarCollectionBlock {
                                env_var_collection_block_handle,
                            }) if !env_var_collection_block_handle
                                .as_ref(ctx)
                                .is_block_completed() =>
                            {
                                env_var_collection_block_handle
                                    .update(ctx, |block, ctx| block.handle_ctrl_c(ctx));
                            }
                            _ => {}
                        }
                    }

                    let was_new = *original_exchange_count == 0;
                    let was_modified = *final_exchange_count != *original_exchange_count;

                    // Child agents in an orchestration tree are part of the
                    // orchestrator's pill bar and should remain visible even
                    // when they're empty (e.g. failed-to-start, or just
                    // haven't received their first event yet). The auto-
                    // remove below is meant to prune accidentally-opened
                    // empty *root* conversations, not children of an
                    // orchestrator.
                    let is_child_agent = BlocklistAIHistoryModel::as_ref(ctx)
                        .conversation(conversation_id)
                        .is_some_and(|c| c.is_child_agent_conversation());

                    // Delete the conversation if it's unmodified, new, has no init steps,
                    // and isn't a child agent in an orchestration tree.
                    if !was_modified && was_new && !has_init_steps && !is_child_agent {
                        conversation_utils::remove_conversation(
                            *conversation_id,
                            me.view_id,
                            false, // Empty new conversations were never synced to the cloud.
                            ctx,
                        );
                    }

                    // This handles the case where the user has taken over control but the command is still in progress.
                    // We only want to insert an agent view block for long running commands that are completed.
                    let is_exit_due_to_user_takeover_of_lrc =
                        matches!(origin, AgentViewEntryOrigin::LongRunningCommand) && {
                            let model = me.model.lock();
                            let active_block = model.block_list().active_block();
                            active_block.is_active_and_long_running()
                        };

                    // LRC conversations should only have one entry point (the original LRC block).
                    let has_existing_lrc_block =
                        me.has_existing_lrc_agent_view_block(*conversation_id);

                    let should_insert = (!me
                        .last_visible_item_is_agent_view_block_for_conversation(*conversation_id)
                        && (has_init_steps || was_modified)
                        && !is_exit_due_to_user_takeover_of_lrc
                        && !has_existing_lrc_block)
                        // If the agent view was entered via accepting a 'new conversation
                        // speedbump', an entry block should always be inserted.
                        || matches!(origin, AgentViewEntryOrigin::AgentRequestedNewConversation);
                    if should_insert {
                        me.insert_agent_view_entry_block(
                            AgentViewEntryBlockParams {
                                conversation_id: *conversation_id,
                                is_new: was_new,
                                is_restored: false, /* is_restored */
                                origin: *origin,
                                agent_view_controller: me.agent_view_controller.clone(),
                            },
                            RichContentInsertionPosition::Append {
                                insert_below_long_running_block: true,
                            },
                            ctx,
                        );
                    }

                    let active_conversation_id = me
                        .agent_view_controller
                        .as_ref(ctx)
                        .agent_view_state()
                        .active_conversation_id();
                    let pending_user_query_conversation_id =
                        me.pending_user_query_conversation_id();
                    let should_keep_pending_user_query = active_conversation_id.is_some()
                        && active_conversation_id == pending_user_query_conversation_id;

                    // Keep the pending query only when the user is still viewing the conversation
                    // targeted by that pending query; otherwise cancel it.
                    if !should_keep_pending_user_query {
                        me.remove_pending_user_query_block(ctx);
                    }
                    me.maybe_run_pending_cloud_mode_start_callback(ctx);

                    ctx.notify();
                }
                AgentViewControllerEvent::ExitConfirmed { .. } => {}
            }
            // Entering or exiting agent view changes whether we need git
            // status updates.
            me.update_git_status_subscription(ctx);
            me.update_pane_configuration(ctx);
            me.update_agent_view_pane_header(ctx);

            // Mark all AgentViewEntry and AIBlock rich content as dirty so their heights get
            // re-measured. When the agent view is active, AgentViewEntryBlock renders as Empty
            // (0 height). When exiting, we need to force a re-layout so the block's actual
            // height is restored. The dirty item processing happens before viewport iteration,
            // so this works even for 0-height items at the prefix of the blocklist.
            let mut model = me.model.lock();
            me.mark_all_rich_content_items_dirty_where(&mut model, |metadata| {
                matches!(
                    metadata,
                    RichContentMetadata::AgentViewEntry(_) | RichContentMetadata::AIBlock(_)
                )
            });
            ctx.notify();
        });

        let ai_context_model = ctx.add_model(|ctx| {
            BlocklistAIContextModel::new(
                sessions.clone(),
                &model_events_handle,
                model.clone(),
                terminal_view_id,
                agent_view_controller.clone(),
                ctx,
            )
        });
        let ai_input_model = ctx.add_model(|ctx| {
            let mut model = BlocklistAIInputModel::new(
                model.clone(),
                agent_view_controller.clone(),
                ai_context_model.clone(),
                terminal_view_id,
                ctx,
            );

            // If NLD is disabled, restore any input config that was saved.
            if !model.is_autodetection_enabled_for_current_context(ctx) {
                if let Some(input_config) = initial_input_config {
                    let is_input_buffer_empty = true;
                    model.set_input_config(input_config, is_input_buffer_empty, None, ctx);
                }
            }
            model
        });

        let get_relevant_files_controller = ctx.add_model(GetRelevantFilesController::new);
        let ai_action_model = ctx.add_model(|ctx| {
            BlocklistAIActionModel::new(
                model.clone(),
                active_session.clone(),
                &model_events_handle,
                get_relevant_files_controller.clone(),
                terminal_view_id,
                ctx,
            )
        });
        let ai_controller = ctx.add_model(|ctx| {
            BlocklistAIController::new(
                ai_input_model.clone(),
                ai_context_model.clone(),
                ai_action_model.clone(),
                active_session.clone(),
                agent_view_controller.clone(),
                model.clone(),
                terminal_view_id,
                ctx,
            )
        });
        let maa_passive_suggestions_model = ctx.add_model(|ctx| {
            MaaPassiveSuggestionsModel::new(
                active_session.clone(),
                model.clone(),
                ai_controller.clone(),
                &model_events_handle,
                ambient_agent_view_model.clone(),
                terminal_view_id,
                ctx,
            )
        });
        ctx.subscribe_to_model(
            &maa_passive_suggestions_model,
            Self::handle_maa_passive_suggestions_event,
        );
        let legacy_passive_suggestions_model = ctx.add_model(|ctx| {
            LegacyPassiveSuggestionsModel::new(
                active_session.clone(),
                model.clone(),
                ai_controller.clone(),
                &model_events_handle,
                terminal_view_id,
                ctx,
            )
        });
        ctx.subscribe_to_model(
            &legacy_passive_suggestions_model,
            Self::handle_legacy_passive_suggestions_event,
        );
        let passive_suggestions_models = PassiveSuggestionsModels {
            maa: maa_passive_suggestions_model,
            legacy: legacy_passive_suggestions_model,
        };

        let find_model = ctx.add_model(|ctx| TerminalFindModel::new(model.clone(), ctx));

        ctx.subscribe_to_model(
            &TerminalSettings::handle(ctx),
            |me, terminal_settings, event, ctx| match event {
                TerminalSettingsChangedEvent::MaximumGridSize { .. } => {
                    let mut model = me.model.lock();
                    model.update_max_grid_size(
                        *terminal_settings.as_ref(ctx).maximum_grid_size.value(),
                    );
                }
                TerminalSettingsChangedEvent::Spacing { .. } => {
                    let appearance = Appearance::as_ref(ctx);
                    let terminal_spacing = terminal_settings
                        .as_ref(ctx)
                        .terminal_spacing(appearance.line_height_ratio(), ctx);
                    me.model.lock().update_blockheight_items(
                        terminal_spacing.block_padding,
                        terminal_spacing.subshell_separator_height,
                    );
                    ctx.notify();
                }
                TerminalSettingsChangedEvent::AltScreenPadding { .. } => {
                    if me.model.lock().is_alt_screen_active() {
                        me.refresh_size(ctx);
                    }
                }
                _ => {}
            },
        );

        ctx.subscribe_to_model(&PaneSettings::handle(ctx), |_, _, event, ctx| {
            if matches!(
                event,
                PaneSettingsChangedEvent::ShouldDimInactivePanes { .. }
            ) {
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(
            &Appearance::handle(ctx),
            move |me, _, event, ctx| match event {
                AppearanceEvent::ThemeChanged => {
                    me.handle_theme_change(ctx);
                }
                AppearanceEvent::MonospaceFontSizeChanged { .. }
                | AppearanceEvent::LineHeightRatioChanged { .. }
                | AppearanceEvent::MonospaceFontFamilyChanged { .. }
                | AppearanceEvent::MonospaceFontWeightChanged { .. }
                | AppearanceEvent::UiFontFamilyChanged { .. } => {
                    me.refresh_size(ctx);
                }
            },
        );

        ctx.subscribe_to_model(&FontSettings::handle(ctx), |_, _, event, ctx| {
            if matches!(
                event,
                FontSettingsChangedEvent::EnforceMinimumContrast { .. }
            ) {
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(&GeneralSettings::handle(ctx), move |_, _, _, ctx| {
            ctx.notify();
        });

        ctx.subscribe_to_model(&InputModeSettings::handle(ctx), |me, _, event, ctx| {
            if matches!(event, InputModeSettingsChangedEvent::InputModeState { .. }) {
                let current_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
                if current_mode == InputMode::Waterfall {
                    // Run the resize logic when switching into Waterfall to potentially update the gap size.
                    me.refresh_size(ctx);
                }

                me.model
                    .lock()
                    .block_list_mut()
                    .set_is_inverted(current_mode.is_inverted_blocklist());

                ctx.notify();
            }
        });

        ctx.subscribe_to_model(
            &DebugSettings::handle(ctx),
            |me, debug_settings, event, ctx| {
                if let DebugSettingsChangedEvent::ShowMemoryStats { .. } = event {
                    me.model.lock().block_list_mut().set_show_memory_stats(
                        debug_settings.as_ref(ctx).should_show_memory_stats(),
                    );
                }
            },
        );

        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, event, ctx| {
            if matches!(event, UserWorkspacesEvent::TeamsChanged) {
                me.update_focused_terminal_info(ctx);
            }
        });

        let (resize_tx, resize_rx) = async_channel::unbounded();
        let (find_link_tx, find_link_rx) = async_channel::unbounded();
        ctx.subscribe_to_model(&model_events_handle, |me, _, event, ctx| {
            me.handle_terminal_event(event, ctx);
        });

        ctx.subscribe_to_model(&ai_controller, |me, handle, event, ctx| {
            me.handle_ai_controller_event(handle, event, ctx);
            // Refresh the conversation details panel when agent output completes
            // (may include new artifacts, run time, credits). This applies to both
            // cloud-task-backed and local AI conversations as long as the panel is open.
            if matches!(
                event,
                BlocklistAIControllerEvent::FinishedReceivingOutput { .. }
            ) && me.is_conversation_details_panel_open
            {
                me.fetch_and_update_conversation_details_panel(ctx);
            }
        });

        // Subscribe to agent conversations model for task status updates
        ctx.subscribe_to_model(
            &AgentConversationsModel::handle(ctx),
            |me, _, event, ctx| {
                let is_task_update = matches!(
                    event,
                    AgentConversationsModelEvent::TasksUpdated
                        | AgentConversationsModelEvent::NewTasksReceived
                );
                if is_task_update {
                    me.maybe_insert_tombstone_for_non_running_shared_ambient_task(ctx);
                }
                let should_refresh_details_panel = matches!(
                    event,
                    AgentConversationsModelEvent::TasksUpdated
                        | AgentConversationsModelEvent::NewTasksReceived
                        | AgentConversationsModelEvent::ConversationUpdated { .. }
                        | AgentConversationsModelEvent::ConversationArtifactsUpdated { .. }
                );
                // Only refresh panel if it's currently open (avoids unnecessary work)
                if should_refresh_details_panel
                    && me.is_conversation_details_panel_open
                    && me
                        .ambient_agent_view_model
                        .as_ref()
                        .is_some_and(|model| model.as_ref(ctx).is_ambient_agent())
                {
                    me.fetch_and_update_conversation_details_panel(ctx);
                    ctx.notify();
                }
            },
        );

        let _ = ctx.spawn_stream_local(
            throttle(WAKEUP_THROTTLE_PERIOD, wakeups_rx),
            Self::handle_terminal_wakeup,
            |_, _| {}, /* on_done */
        );

        let _ = ctx.spawn_stream_local(
            debounce(DEBOUNCE_PERIOD, find_link_rx),
            Self::handle_find_link,
            |_, _| {}, /* on_done */
        );

        let _ = ctx.spawn_stream_local(resize_rx, Self::after_terminal_view_layout, |_, _| {});

        let menu_positioning_provider = Arc::new(TerminalViewMenuPositioningProvider {
            parent: ctx.handle(),
        });

        let cli_subagent_controller = ctx.add_model(|ctx| {
            CLISubagentController::new(
                &ai_controller,
                &ai_action_model,
                if FeatureFlag::AgentView.is_enabled() {
                    Some(agent_view_controller.clone())
                } else {
                    None
                },
                model.clone(),
                &model_events_handle,
                terminal_view_id,
                ctx,
            )
        });
        ctx.subscribe_to_model(
            &cli_subagent_controller,
            Self::handle_cli_subagent_controller_event,
        );
        let terminal_content_element_position_id =
            format!("terminal_content_element_{}", ctx.view_id());

        let input: ViewHandle<Input> = ctx.add_typed_action_view(|ctx| {
            Input::new(
                model.clone(),
                resources.tips_completed.clone(),
                resources.server_api.clone(),
                sessions.clone(),
                size_info,
                menu_positioning_provider,
                current_prompt.clone(),
                ai_controller.clone(),
                ai_context_model.clone(),
                ai_input_model.clone(),
                ai_action_model.clone(),
                cli_subagent_controller.clone(),
                terminal_view_id,
                None, // current_repo_path - will be set when CWD is determined
                model_events_handle.clone(),
                agent_view_controller.clone(),
                ambient_agent_view_model.clone(),
                active_session.clone(),
                ephemeral_message_model.clone(),
                ctx,
            )
        });

        let inline_menu_positioner = input.as_ref(ctx).inline_terminal_menu_positioner().clone();
        ctx.subscribe_to_model(&inline_menu_positioner, |_, _, _, ctx| {
            ctx.notify();
        });
        let suggestions_mode_model = input.as_ref(ctx).suggestions_mode_model().clone();
        ctx.subscribe_to_model(&suggestions_mode_model, |_, _, _, ctx| {
            ctx.notify();
        });

        let input_position_id = input.read(ctx, |input, _| input.save_position_id());
        ctx.subscribe_to_view(&input, move |me, _, event, ctx| {
            me.handle_input_event(event, ctx);
        });

        let ai_status_bar = input.as_ref(ctx).agent_status_bar().clone();
        ctx.subscribe_to_view(&ai_status_bar, |me, _, event, ctx| match event {
            BlocklistAIStatusBarEvent::SummarizationCancelDialogToggled { is_open } => {
                me.pane_configuration.update(ctx, |pane_config, ctx| {
                    pane_config.set_has_open_modal(*is_open, ctx)
                });
                ctx.emit(Event::SummarizationCancelDialogToggled { is_open: *is_open });
            }
            BlocklistAIStatusBarEvent::Stop => me.ctrl_c(ctx),
        });
        if let Some(ambient_agent_view_model) = ambient_agent_view_model.as_ref() {
            ctx.subscribe_to_model(ambient_agent_view_model, |me, _, event, ctx| {
                me.handle_ambient_agent_event(event, ctx);
            });
        }

        let ai_render_context = Rc::new(RefCell::new(BlocklistAIRenderContext {
            block_ids: HashMap::from_iter([
                (
                    AIContextInclusionState::Pending,
                    ai_context_model
                        .as_ref(ctx)
                        .pending_context_block_ids()
                        .clone(),
                ),
                (AIContextInclusionState::Active, Default::default()),
            ]),
            selected_conversation_id: None,
            exchange_ids: None,
            should_highlight_context: false,
            is_ai_input_enabled: ai_input_model.as_ref(ctx).is_ai_input_enabled(),
            has_pending_context_selected_text: ai_context_model
                .as_ref(ctx)
                .pending_context_selected_text()
                .is_some(),
        }));

        ctx.subscribe_to_model(&ai_context_model, Self::handle_ai_context_model_event);
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            Self::handle_ai_history_model_event,
        );
        ctx.subscribe_to_model(&ai_input_model, Self::handle_ai_input_model_event);
        ctx.subscribe_to_model(&ai_action_model, Self::handle_ai_action_model_event);
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), |me, _, event, ctx| {
            if let CLIAgentSessionsModelEvent::Ended {
                terminal_view_id, ..
            } = event
            {
                if *terminal_view_id == me.view_id
                    && me.auto_stop_sharing_on_cli_end
                    && me.model.lock().shared_session_status().is_active_sharer()
                {
                    me.auto_stop_sharing_on_cli_end = false;
                    me.stop_sharing_session(SharedSessionActionSource::NonUser, ctx);
                }
            }
            me.handle_cli_agent_sessions_event(event, ctx)
        });
        ctx.subscribe_to_model(
            &ai_action_model.as_ref(ctx).shell_command_executor(ctx),
            Self::handle_shell_command_executor_event,
        );

        ctx.subscribe_to_model(
            &ai_action_model.as_ref(ctx).start_agent_executor(ctx),
            Self::handle_start_agent_executor_event,
        );
        let find_bar = ctx.add_typed_action_view(|ctx| Find::new(find_model.clone(), ctx));
        ctx.subscribe_to_view(&find_bar, move |me, _, event, ctx| {
            me.handle_find_event(event, ctx);
        });

        let block_filter_editor = ctx.add_typed_action_view(BlockFilterEditor::new);
        ctx.subscribe_to_view(&block_filter_editor, move |me, _, event, ctx| {
            me.handle_block_filter_event(event, ctx);
        });

        let context_menu = ctx.add_typed_action_view(|_| {
            Menu::new()
                .prevent_interaction_with_other_elements()
                .with_drop_shadow()
        });
        ctx.subscribe_to_view(&context_menu, move |me, _, event, ctx| {
            me.handle_menu_event(event, ctx);
        });

        let slow_bootstrap_banner = ctx.add_typed_action_view(|_| {
            Banner::<TerminalAction>::new_with_buttons(
                BannerTextContent::formatted_text(vec![
                    FormattedTextFragment::plain_text(
                        "Seems like your shell is taking a while to start...  ",
                    ),
                    FormattedTextFragment::hyperlink("More info", KNOWN_ISSUES_URL),
                ]),
                vec![BannerTextButton::new(
                    "Show initialization block".to_string(),
                    Rc::new(|event_ctx, _ctx, _position| {
                        event_ctx.dispatch_typed_action(BannerAction::<TerminalAction>::Action(
                            TerminalAction::ShowInitializationBlock,
                        ));
                    }),
                )],
                true,
            )
        });
        ctx.subscribe_to_view(&slow_bootstrap_banner, |me, _, event, ctx| {
            me.handle_slow_bootstrap_banner_event(event, ctx);
        });

        ctx.subscribe_to_model(&sessions, |me, _, event, ctx| {
            me.handle_sessions_event(event.clone(), ctx);
        });

        let control_master_error_banner = ctx.add_typed_action_view(|_| {
            Banner::new(BannerTextContent::formatted_text(vec![
                FormattedTextFragment::plain_text("Seems like your completions are not working ("),
                FormattedTextFragment::hyperlink("more info", CONTROLMASTER_ISSUES_URL),
                FormattedTextFragment::plain_text("). Enabling the SSH extension in "),
                FormattedTextFragment::hyperlink_action(
                    "settings",
                    TerminalAction::ShowWarpifySettings,
                ),
                FormattedTextFragment::plain_text(" may resolve this issue."),
            ]))
        });

        ctx.subscribe_to_view(&control_master_error_banner, |me, _, event, ctx| {
            me.handle_controlmaster_error_banner_event(event, ctx);
        });

        let incompatible_configuration_banner = ctx.add_typed_action_view(|_| {
            Banner::new(BannerTextContent::formatted_text(vec![
                FormattedTextFragment::plain_text(
                    "Your shell configuration is incompatible with Warp...  ",
                ),
                FormattedTextFragment::hyperlink("More info", KNOWN_ISSUES_URL),
            ]))
        });

        ctx.subscribe_to_view(&incompatible_configuration_banner, |me, _, event, ctx| {
            me.handle_incompatible_configuration_banner_event(event, ctx);
        });

        let emacs_bindings_banner = ctx.add_typed_action_view(|_| {
            Banner::new_with_buttons(
                BannerTextContent::formatted_text(vec![
                    FormattedTextFragment::plain_text("Did you intend "),
                    FormattedTextFragment::inline_code("ctrl-a"),
                    FormattedTextFragment::plain_text("/"),
                    FormattedTextFragment::inline_code("ctrl-e"),
                    FormattedTextFragment::plain_text(" to move the cursor?"),
                ]),
                // Here, we use DismissalType::Temporary and DismissalType::Permanent variants
                // as stand-ins for changing bindings vs. leaving them as-is.
                // TODO(Linear PLAT-512): update Banner to support generic event type.
                vec![
                    BannerTextButton::new(
                        String::from("Yes, use Emacs-style bindings"),
                        Rc::new(|event_ctx, _app_ctx, _| {
                            event_ctx.dispatch_typed_action(
                                BannerAction::<TerminalAction>::Dismiss(DismissalType::Temporary),
                            );
                        }),
                    ),
                    BannerTextButton::new(
                        String::from("No, keep IDE bindings"),
                        Rc::new(|event_ctx, _app_ctx, _| {
                            event_ctx.dispatch_typed_action(
                                BannerAction::<TerminalAction>::Dismiss(DismissalType::Permanent),
                            );
                        }),
                    ),
                ],
                /* with_close_button */ false,
            )
            .with_icon(icons::Icon::HelpCircle)
        });

        if OperatingSystem::get().is_linux() {
            ctx.subscribe_to_view(&emacs_bindings_banner, |me, _, event, ctx| {
                me.handle_emacs_bindings_banner_clicked(event, ctx);
            });
        }

        let windowing_state_handle = WindowManager::handle(ctx);
        ctx.subscribe_to_model(&windowing_state_handle, |me, _handle, evt, ctx| match evt {
            windowing::StateEvent::ValueChanged { current, previous } => {
                me.handle_windowing_state_update((current, previous), ctx);
            }
        });

        let ligature_handle = LigatureSettings::handle(ctx);
        ctx.subscribe_to_model(&ligature_handle, |_, _, _, ctx| ctx.notify());

        let privacy_settings_handle = PrivacySettings::handle(ctx);
        ctx.subscribe_to_model(
            &privacy_settings_handle,
            |me, privacy_settings_handle, event, ctx| {
                if let PrivacySettingsChangedEvent::UpdateIsTelemetryEnabled { .. } = event {
                    me.privacy_settings_snapshot =
                        privacy_settings_handle.as_ref(ctx).get_snapshot(ctx)
                }
            },
        );

        let block_visibility_settings_handle = BlockVisibilitySettings::handle(ctx);
        ctx.subscribe_to_model(
            &block_visibility_settings_handle,
            |me, block_visibility_settings_handle, event, ctx| match event {
                BlockVisibilitySettingsChangedEvent::ShouldShowBootstrapBlock { .. } => {
                    let should_show_bootstrap_block = *block_visibility_settings_handle
                        .as_ref(ctx)
                        .should_show_bootstrap_block
                        .value();
                    let mut model = me.model.lock();
                    model
                        .block_list_mut()
                        .set_show_bootstrap_block(should_show_bootstrap_block);
                    ctx.notify();
                }
                BlockVisibilitySettingsChangedEvent::ShouldShowInBandCommandBlocks { .. } => {
                    let should_show_in_band_command_blocks = *block_visibility_settings_handle
                        .as_ref(ctx)
                        .should_show_in_band_command_blocks
                        .value();
                    let mut model = me.model.lock();
                    model
                        .block_list_mut()
                        .set_show_in_band_command_blocks(should_show_in_band_command_blocks);
                    ctx.notify();
                }
                BlockVisibilitySettingsChangedEvent::ShouldShowSSHBlock { .. } => {}
            },
        );

        let block_list_settings_handle = BlockListSettings::handle(ctx);
        ctx.subscribe_to_model(&block_list_settings_handle, |_, _, evt, ctx| match evt {
            BlockListSettingsChangedEvent::ShowJumpToBottomOfBlockButton { .. } => ctx.notify(),
            BlockListSettingsChangedEvent::SnackbarEnabled { .. } => ctx.notify(),
            BlockListSettingsChangedEvent::ShowBlockDividers { .. } => ctx.notify(),
        });

        ctx.subscribe_to_model(&SessionSettings::handle(ctx), move |me, _, evt, ctx| {
            me.handle_session_settings_event(evt, ctx);
        });

        // Re-evaluate git status subscription when the prompt configuration
        // changes (e.g. chips added/removed, input type toggled).
        ctx.subscribe_to_model(&Prompt::handle(ctx), |me, _, _, ctx| {
            me.update_git_status_subscription(ctx);
        });

        ctx.subscribe_to_model(&AltScreenReporting::handle(ctx), move |me, _, evt, ctx| {
            me.handle_reporting_settings_event(evt, ctx);
        });

        let initial_title = model.lock().shell_launch_state().display_name().to_string();

        let pane_configuration = ctx.add_model(|_| PaneConfiguration::new(initial_title));

        ctx.observe(
            &WindowActiveSession::handle(ctx),
            |me, active_session, ctx| {
                let active_session = active_session.as_ref(ctx);
                let state =
                    if active_session.terminal_view_id(ctx.window_id()) == Some(ctx.view_id()) {
                        ActiveSessionState::Active
                    } else {
                        ActiveSessionState::Inactive
                    };
                me.set_active_session_state(state, ctx);
            },
        );
        ctx.subscribe_to_model(&KeybindingChangedNotifier::handle(ctx), |me, _, _, ctx| {
            me.cancel_command_keystroke =
                keybinding_name_to_keystroke(CANCEL_COMMAND_KEYBINDING, ctx);

            me.refresh_pane_header(ctx);
            ctx.notify();
        });

        let ssh_file_upload = ctx.add_typed_action_view(|_| FileUpload::new());

        if FeatureFlag::SshDragAndDrop.is_enabled() {
            ctx.subscribe_to_view(&ssh_file_upload, |_terminal, _file_upload, event, ctx| {
                // Pass the file upload events up so they can be processed by the pane group.
                match event {
                    FileUploadEvent::CopyFileToRemote { command, upload_id } => {
                        ctx.emit(Event::CopyFileToRemote {
                            command: command.clone(),
                            upload_id: *upload_id,
                        });
                    }
                    FileUploadEvent::OpenUploadSession(upload_id) => {
                        ctx.emit(Event::OpenFileUploadSession(*upload_id));
                    }
                    FileUploadEvent::TerminateUploadSession(upload_id) => {
                        ctx.emit(Event::TerminateFileUploadSession(*upload_id));
                    }
                }
            });
        }

        // Here we initialize the block list mouse states for block zero.
        // Afterwards, we initialize all block list mouse states for a block when the
        // previous block sends a `BlockCompleted` event.
        let mut block_list_mouse_states = BlockListMouseStates::default();
        block_list_mouse_states
            .label_mouse_states
            .entry(BlockIndex::zero())
            .or_default();
        block_list_mouse_states
            .bookmark_mouse_states
            .entry(BlockIndex::zero())
            .or_default();
        block_list_mouse_states
            .filter_mouse_states
            .entry(BlockIndex::zero())
            .or_default();

        ctx.subscribe_to_model(&AssetCache::handle(ctx), |me, _, event, _| match event {
            AssetCacheEvent::ImagesEvicted { image_ids } => {
                let mut terminal_model = me.model.lock();
                for &image_id in image_ids {
                    terminal_model.remove_image_id_to_metadata_entry(image_id);
                }
            }
        });

        let first_time_cloud_agent_setup_view =
            ctx.add_typed_action_view(ambient_agent::FirstTimeCloudAgentSetupView::new);

        ctx.subscribe_to_view(&first_time_cloud_agent_setup_view, |me, _, event, ctx| {
            me.handle_first_time_cloud_agent_setup_event(event, ctx);
        });

        let environment_setup_mode_selector =
            ctx.add_typed_action_view(EnvironmentSetupModeSelector::new);

        ctx.subscribe_to_view(&environment_setup_mode_selector, |me, _, event, ctx| {
            me.handle_environment_setup_mode_selector_event(event, ctx);
        });

        if FeatureFlag::CodebaseIndexSpeedbump.is_enabled() {
            // Check whether or not to show the codebase index speedbump when the codebase indexing settings change.
            ctx.subscribe_to_model(&CodeSettings::handle(ctx), |me, _, _, ctx| {
                me.check_codebase_index_speedbump_on_settings_changed(ctx);
            });

            // Check whether or not to show the codebase index speedbump when AI settings change.
            ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, ai_settings_event, ctx| {
                match ai_settings_event {
                    AISettingsChangedEvent::IsAnyAIEnabled { .. }
                    | AISettingsChangedEvent::AgentModeCodingPermissions { .. }
                    | AISettingsChangedEvent::AgentModeCodingFileReadAllowlist { .. } => {
                        me.check_codebase_index_speedbump_on_settings_changed(ctx);
                    }
                    _ => {}
                }
            });
        }

        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, ai_settings_event, ctx| {
            if let AISettingsChangedEvent::AwsBedrockCredentialsEnabled { .. } = ai_settings_event {
                if !UserWorkspaces::as_ref(ctx).is_aws_bedrock_credentials_enabled(ctx) {
                    me.remove_aws_bedrock_login_banner(ctx);
                }
            }
        });

        let agent_todos_popup = Self::build_agent_todos_popup(ai_context_model.clone(), ctx);

        let terminal_view_id = ctx.view_id();
        let agent_input_footer = input.as_ref(ctx).agent_input_footer().clone();
        let use_agent_button_bar = ctx.add_typed_action_view(|ctx| {
            UseAgentToolbar::new(
                terminal_view_id,
                model.clone(),
                &model_events_handle,
                agent_input_footer.clone(),
                ctx,
            )
        });
        let orchestration_pill_bar = ctx.add_typed_action_view(|ctx| {
            OrchestrationPillBar::new(agent_view_controller.clone(), ctx)
        });
        ctx.subscribe_to_view(&orchestration_pill_bar, |_, _, _, ctx| ctx.notify());

        let agent_view_back_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("for terminal", AgentViewHeaderTheme)
                .with_icon(icons::Icon::ArrowLeft)
                .with_size(ButtonSize::Small)
                .with_keybinding(
                    KeystrokeSource::Fixed(Keystroke {
                        key: "escape".to_string(),
                        ..Default::default()
                    }),
                    ctx,
                )
                .with_disabled_theme(AgentViewHeaderDisabledTheme)
                .with_keybinding_before_label(true)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(
                        PaneHeaderAction::<TerminalAction, TerminalAction>::CustomAction(
                            TerminalAction::ExitAgentView,
                        ),
                    )
                })
        });

        // Conversation details panel (cloud Oz runs and any active local AI conversation).
        let conversation_details_panel = ctx.add_typed_action_view(|ctx| {
            crate::ai::conversation_details_panel::ConversationDetailsPanel::new(
                false, // don't show "Open" button since we're already viewing the conversation
                320.0, // initial width
                ctx,
            )
        });
        ctx.subscribe_to_view(&conversation_details_panel, |me, _, event, ctx| {
            match event {
                ConversationDetailsPanelEvent::Close => {
                    me.is_conversation_details_panel_open = false;
                    ctx.notify();
                }
                ConversationDetailsPanelEvent::OpenPlanNotebook { notebook_uid } => {
                    // Convert NotebookId -> SyncId -> ObjectUid (String)
                    let object_uid = SyncId::from(*notebook_uid).uid();
                    ctx.emit(Event::OpenWarpDriveObjectInPane(object_uid));
                }
            }
        });

        let window_id = ctx.window_id();
        let mut terminal_view = Self {
            model,
            input,
            inline_menu_positioner,
            view_handle: ctx.handle(),
            size_info: size_info.into(),
            snackbar_header_state: Default::default(),
            colors,
            scroll_position: ScrollState::new(ScrollPosition::FollowsBottomOfMostRecentBlock),
            scroll_position_before_entering_agent_view: None,
            blocklist_vertical_scroll_state: Default::default(),
            alt_screen_vertical_scroll_state: Default::default(),
            alt_screen_scroll_top: Lines::zero(),
            horizontal_clipped_scroll_state: Default::default(),
            is_selecting: false,
            context_menu_state: None,
            context_menu,
            hovered_secret: None,
            open_secret_tool_tip: None,
            hovered_block_index: None,
            selected_blocks: Default::default(),
            block_list_mouse_states,
            any_session_contains_remote_blocks: false,
            any_session_contains_restored_remote_blocks: false,
            mouse_down_block_index: None,
            mouse_states: Default::default(),
            open_grid_link_tool_tip: None,
            open_rich_content_link_tool_tip: None,
            server_api: resources.server_api.clone(),
            auth_state: AuthStateProvider::as_ref(ctx).get().clone(),
            find_bar,
            resize_tx,
            find_link_tx,
            highlighted_link: HighlightedLinkOption::default(),
            last_hover_fragment_boundary: None,
            bootstrap_start: None,
            is_login_shell_bootstrapped: false,
            awaiting_pending_command_completion: false,
            pending_command_queue: Default::default(),
            enter_agent_view_after_pending_commands: false,
            slow_bootstrap_banner,
            is_slow_bootstrap_banner_open: false,
            incompatible_configuration_banner,
            is_incompatible_configuration_banner_open: false,
            emacs_bindings_banner,
            is_emacs_bindings_banner_open: false,
            control_master_error_banner,
            control_master_error_banner_state: Default::default(),
            pane_configuration,
            focus_handle: None,
            sessions,
            remote_server_shimmer_handle: ShimmeringTextStateHandle::new(),
            active_block_metadata: None,
            block_text_selection_start_position: None,
            background_executor: ctx.background_executor().clone(),
            inline_banners_state: Default::default(),
            bookmarked_blocks: Default::default(),
            file_link_scanning_join_handle: None,
            last_focus_ts: None,
            tips_completed: resources.tips_completed.clone(),
            privacy_settings_snapshot: privacy_settings_handle.as_ref(ctx).get_snapshot(ctx),
            was_ever_visible: false,
            view_id: ctx.view_id(),
            current_state: TerminalViewStateChange::default(),
            did_notify_long_running: false,
            is_focused_and_active: true,
            current_prompt,
            model_event_sender,
            block_filter_editor,
            active_filter_editor_block_index: None,
            rich_content_views: Vec::new(),
            pending_user_query_view_id: None,
            pending_user_query_kind: None,
            queued_prompt_callback: None,
            usage_footer_view_ids: Default::default(),
            block_onboarding_active: false,
            onboarding_agentic_suggestions_block: None,
            onboarding_prompt_block: None,
            settings_import_onboarding_block: None,
            onboarding_callout_view: None,
            pending_onboarding_agentic_suggestions_block: true,
            pending_auto_bootstrap_shell_type: None,
            pending_env_var_collection: None,
            env_vars: Vec::new(),
            show_snackbar: true,
            hover_near_snackbar_area: false,
            ai_controller,
            passive_suggestions_models,
            ai_action_model,
            ai_render_context,
            get_relevant_files_controller,
            shared_session: None,
            pending_share_source: None,
            auto_stop_sharing_on_cli_end: false,
            conversation_ended_tombstone_view_id: None,
            ai_input_model,
            ai_context_model,
            window_id,
            content_element_position_id: terminal_content_element_position_id,
            input_position_id,
            input_hoverable_handle: Default::default(),
            find_model,
            warpify_state: Default::default(),
            cancel_command_keystroke: keybinding_name_to_keystroke(CANCEL_COMMAND_KEYBINDING, ctx),
            is_file_drop_target: false,
            is_ssh_file_uploader: false,
            ssh_file_upload,
            most_recent_command_correction: None,
            shell_indicator_type: None,
            shell_detail: None,
            position_id: format!("terminal_view_{}", ctx.view_id()),
            cursor_position_id: format!("terminal_view:cursor_{}", ctx.view_id()),
            active_session,
            pty_spawn_failed: false,
            model_events_handle,
            is_todo_popup_visible: false,
            agent_todos_popup,
            #[cfg(feature = "local_fs")]
            git_repo_status: None,
            #[cfg(feature = "local_fs")]
            deferred_code_review_open: None,
            block_completed_callbacks: Default::default(),
            conversation_completed_callbacks: Default::default(),
            current_repo_path: None,
            terminal_title: Default::default(),
            ignore_next_set_title_event: false,
            cli_subagent_views: Default::default(),
            cli_subagent_controller,
            use_agent_footer: use_agent_button_bar,
            agent_view_controller,
            agent_view_back_button,
            orchestration_pill_bar,
            is_orchestration_split_off: false,
            is_using_conversation_for_pane_header_title: false,
            ambient_agent_view_model,
            conversation_details_panel,
            is_conversation_details_panel_open: false,
            has_auto_opened_conversation_details_panel: false,
            conversation_details_panel_auto_open_policy: Default::default(),
            pending_cloud_followup_task_id: None,
            #[cfg(not(target_arch = "wasm32"))]
            conversation_details_panel_toggle_mouse_state: Default::default(),
            ambient_agent_cancel_mouse_state: Default::default(),
            active_init_project_model: None,
            is_pending_aws_login: false,
            manual_pty_shutdown_requested: false,
            first_time_cloud_agent_setup_view,
            environment_setup_mode_selector,
            is_environment_setup_mode_selector_open: false,
            pane_stack: None,
            pending_cloud_mode_start_callback: None,
            pending_cloud_mode_start_abort_handle: None,
            ephemeral_message_model,
            pty_recorder: ctx
                .add_model(|ctx| PtyRecorder::new(inactive_pty_reads_rx, window_id, ctx)),
            active_viewer_driven_size: None,
        };
        terminal_view.register_subscriptions_for_use_agent_footer(ctx);

        // Forward RemoteServerManager setup events into the terminal event stream
        // so the ModelEventDispatcher can gate session initialization on them.
        if FeatureFlag::SshRemoteServer.is_enabled() {
            let mgr_handle = RemoteServerManager::handle(ctx);
            ctx.subscribe_to_model(&mgr_handle, |me, _, event, ctx| {
                // `RemoteServerManager` is a singleton, so every `TerminalView` receives every event.
                // Filter for session-scoped events that are specifically tracked by this view.
                // Host-scoped variants return `None` and pass through unfiltered.
                if let Some(sid) = event.session_id() {
                    if !me.sessions.as_ref(ctx).tracks_session(sid) {
                        return;
                    }
                }
                match event {
                    RemoteServerManagerEvent::SetupStateChanged { .. } => {
                        // Sessions handles the state update directly via its own
                        // subscription to the manager. Notify the view so the
                        // loading footer re-renders with the updated message.
                        ctx.notify();
                    }
                    RemoteServerManagerEvent::SessionConnected { session_id, .. } => {
                        me.model.lock().event_proxy.send_terminal_event(
                            crate::terminal::event::Event::RemoteServerReady {
                                session_id: *session_id,
                            },
                        );
                        let (remote_os, remote_arch) = RemoteServerManager::handle(ctx)
                            .as_ref(ctx)
                            .platform_for_session(*session_id)
                            .map(|p| {
                                (
                                    Some(p.os.as_str().to_owned()),
                                    Some(p.arch.as_str().to_owned()),
                                )
                            })
                            .unwrap_or((None, None));
                        send_telemetry_from_ctx!(
                            TelemetryEvent::RemoteServerInitialization {
                                phase: RemoteServerInitPhase::Initialize,
                                error: None,
                                remote_os,
                                remote_arch,
                                exit_code: None,
                                signal_killed: None,
                                proxy_stderr: None,
                            },
                            ctx
                        );
                    }
                    RemoteServerManagerEvent::SessionConnectionFailed {
                        session_id,
                        phase,
                        error,
                        exit_status,
                        proxy_stderr,
                        is_cancelled,
                    } => {
                        me.model.lock().event_proxy.send_terminal_event(
                            crate::terminal::event::Event::RemoteServerFailed {
                                session_id: *session_id,
                                error: error.clone(),
                            },
                        );

                        if !is_cancelled {
                            let (remote_os, remote_arch) = RemoteServerManager::handle(ctx)
                                .as_ref(ctx)
                                .platform_for_session(*session_id)
                                .map(|p| {
                                    (
                                        Some(p.os.as_str().to_owned()),
                                        Some(p.arch.as_str().to_owned()),
                                    )
                                })
                                .unwrap_or((None, None));
                            send_telemetry_from_ctx!(
                                TelemetryEvent::RemoteServerInitialization {
                                    phase: *phase,
                                    error: Some(error.clone()),
                                    remote_os,
                                    remote_arch,
                                    exit_code: exit_status.as_ref().and_then(|s| s.code),
                                    signal_killed: exit_status.as_ref().map(|s| s.signal_killed),
                                    proxy_stderr: proxy_stderr.clone(),
                                },
                                ctx
                            );
                            me.show_ssh_remote_server_failed_banner(
                                *session_id,
                                remote_server::transport::UserFacingError {
                                    body: "Failed to start SSH extension".into(),
                                    detail: if error.is_empty() {
                                        None
                                    } else {
                                        Some(error.clone())
                                    },
                                },
                                ctx,
                            );
                        }
                    }
                    RemoteServerManagerEvent::SessionDisconnected {
                        session_id,
                        exit_status,
                        was_reconnect_attempt,
                        ..
                    } => {
                        let (remote_os, remote_arch) = RemoteServerManager::handle(ctx)
                            .as_ref(ctx)
                            .platform_for_session(*session_id)
                            .map(|p| {
                                (
                                    Some(p.os.as_str().to_owned()),
                                    Some(p.arch.as_str().to_owned()),
                                )
                            })
                            .unwrap_or((None, None));
                        if *was_reconnect_attempt {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::RemoteServerReconnectExhausted {
                                    attempts: remote_server::manager::MAX_RECONNECT_ATTEMPTS,
                                    remote_os,
                                    remote_arch,
                                    exit_code: exit_status.as_ref().and_then(|s| s.code),
                                    signal_killed: exit_status.as_ref().map(|s| s.signal_killed),
                                },
                                ctx
                            );
                        } else {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::RemoteServerDisconnection {
                                    remote_os,
                                    remote_arch,
                                },
                                ctx
                            );
                        }
                    }
                    RemoteServerManagerEvent::SessionDeregistered { session_id } => {
                        // Clean up any stale SSH remote-server choice block if the
                        // session disappears (e.g. network drop, Ctrl-C, `exit`)
                        // before the user picks an option.
                        me.remove_ssh_remote_server_choice_block(*session_id, ctx);
                        me.remove_ssh_remote_server_failed_banner(*session_id, ctx);
                    }
                    RemoteServerManagerEvent::BinaryInstallComplete {
                        session_id,
                        result,
                        install_source,
                    } => {
                        let (remote_os, remote_arch) = RemoteServerManager::handle(ctx)
                            .as_ref(ctx)
                            .platform_for_session(*session_id)
                            .map(|p| {
                                (
                                    Some(p.os.as_str().to_owned()),
                                    Some(p.arch.as_str().to_owned()),
                                )
                            })
                            .unwrap_or((None, None));
                        send_telemetry_from_ctx!(
                            TelemetryEvent::RemoteServerInstallation {
                                error: result.as_ref().err().map(|e| e.to_string()),
                                install_source: *install_source,
                                remote_os,
                                remote_arch,
                            },
                            ctx
                        );
                        if let Err(error) = result {
                            log::warn!("Remote server install failed: {error:#}");
                            me.show_ssh_remote_server_failed_banner(
                                *session_id,
                                error.user_facing_error(
                                    remote_server::transport::SetupStage::InstallBinary,
                                ),
                                ctx,
                            );
                        }
                    }
                    RemoteServerManagerEvent::BinaryCheckComplete {
                        session_id,
                        result,
                        remote_platform,
                        ..
                    } => {
                        let (remote_os, remote_arch) = remote_platform
                            .as_ref()
                            .map(|p| {
                                (
                                    Some(p.os.as_str().to_owned()),
                                    Some(p.arch.as_str().to_owned()),
                                )
                            })
                            .unwrap_or((None, None));
                        send_telemetry_from_ctx!(
                            TelemetryEvent::RemoteServerBinaryCheck {
                                found: matches!(result, Ok(true)),
                                error: result.as_ref().err().map(|e| e.to_string()),
                                remote_os,
                                remote_arch,
                            },
                            ctx
                        );
                        if let Err(error) = result {
                            log::warn!("Remote server binary check failed: {error:#}");
                            me.show_ssh_remote_server_failed_banner(
                                *session_id,
                                error.user_facing_error(
                                    remote_server::transport::SetupStage::CheckBinary,
                                ),
                                ctx,
                            );
                        }
                    }
                    RemoteServerManagerEvent::ClientRequestFailed {
                        session_id,
                        operation,
                        error_kind,
                    } => {
                        let (remote_os, remote_arch) = RemoteServerManager::handle(ctx)
                            .as_ref(ctx)
                            .platform_for_session(*session_id)
                            .map(|p| {
                                (
                                    Some(p.os.as_str().to_owned()),
                                    Some(p.arch.as_str().to_owned()),
                                )
                            })
                            .unwrap_or((None, None));
                        send_telemetry_from_ctx!(
                            TelemetryEvent::RemoteServerClientRequestError {
                                operation: *operation,
                                error_type: *error_kind,
                                remote_os,
                                remote_arch,
                            },
                            ctx
                        );
                    }
                    RemoteServerManagerEvent::ServerMessageDecodingError { session_id } => {
                        let (remote_os, remote_arch) = RemoteServerManager::handle(ctx)
                            .as_ref(ctx)
                            .platform_for_session(*session_id)
                            .map(|p| {
                                (
                                    Some(p.os.as_str().to_owned()),
                                    Some(p.arch.as_str().to_owned()),
                                )
                            })
                            .unwrap_or((None, None));
                        send_telemetry_from_ctx!(
                            TelemetryEvent::RemoteServerMessageDecodingError {
                                remote_os,
                                remote_arch,
                            },
                            ctx
                        );
                    }
                    RemoteServerManagerEvent::NavigatedToDirectory {
                        session_id: nav_session_id,
                        remote_path,
                        is_git: _,
                    } => {
                        // Repo registration is now handled by the unified
                        // detect_possible_git_repo callback in BlockMetadataReceived.
                        // Check if this navigation belongs to our active session
                        // using exact session_id match (no CWD heuristics).
                        let is_relevant = me
                            .active_block_session_id()
                            .is_some_and(|sid| sid == *nav_session_id);
                        if is_relevant {
                            ctx.emit(Event::Pane(PaneEvent::RemoteRepoNavigated {
                                remote_path: remote_path.clone(),
                            }));
                        }
                    }
                    RemoteServerManagerEvent::SessionReconnected {
                        session_id,
                        attempt,
                        ..
                    } => {
                        let (remote_os, remote_arch) = RemoteServerManager::handle(ctx)
                            .as_ref(ctx)
                            .platform_for_session(*session_id)
                            .map(|p| {
                                (
                                    Some(p.os.as_str().to_owned()),
                                    Some(p.arch.as_str().to_owned()),
                                )
                            })
                            .unwrap_or((None, None));
                        send_telemetry_from_ctx!(
                            TelemetryEvent::RemoteServerReconnection {
                                attempt: *attempt,
                                remote_os,
                                remote_arch,
                            },
                            ctx
                        );
                    }
                    RemoteServerManagerEvent::HostDisconnected { host_id } => {
                        #[cfg(target_family = "wasm")]
                        let _ = host_id;
                        #[cfg(not(target_family = "wasm"))]
                        DetectedRepositories::handle(ctx).update(ctx, |repos, _| {
                            repos.remove_roots_for_host(host_id);
                        });

                        // Drop and broadcast the stale remote repo so downstream consumers
                        // stop acting on a host with no live client.
                        let matches_host = matches!(
                            me.current_repo_path.as_ref(),
                            Some(LocalOrRemotePath::Remote(rp)) if &rp.host_id == host_id,
                        );
                        if matches_host {
                            me.current_repo_path = None;
                            ctx.emit(Event::Pane(PaneEvent::RepoChanged));
                        }
                    }
                    RemoteServerManagerEvent::SessionConnecting { .. }
                    | RemoteServerManagerEvent::HostConnected { .. }
                    | RemoteServerManagerEvent::RepoMetadataSnapshot { .. }
                    | RemoteServerManagerEvent::RepoMetadataUpdated { .. }
                    | RemoteServerManagerEvent::RepoMetadataDirectoryLoaded { .. }
                    | RemoteServerManagerEvent::CodebaseIndexStatusesSnapshot { .. }
                    | RemoteServerManagerEvent::CodebaseIndexStatusUpdated { .. }
                    | RemoteServerManagerEvent::CodebaseIndexMutationFailed { .. }
                    | RemoteServerManagerEvent::BufferUpdated { .. }
                    | RemoteServerManagerEvent::BufferConflictDetected { .. }
                    | RemoteServerManagerEvent::DiffStateSnapshotReceived { .. }
                    | RemoteServerManagerEvent::DiffStateMetadataUpdateReceived { .. }
                    | RemoteServerManagerEvent::DiffStateFileDeltaReceived { .. }
                    | RemoteServerManagerEvent::GetBranchesResponse { .. } => {}
                }
            });
        }
        terminal_view.any_session_contains_restored_remote_blocks =
            terminal_view.contains_restored_remote_blocks();

        // Restore AI conversations and create AI blocks after terminal view initialization
        if let Some(restoration) = conversation_restoration {
            terminal_view.restore_conversations_on_view_creation(restoration, ctx);
        }

        send_telemetry_from_ctx!(TelemetryEvent::SessionCreation, ctx);

        terminal_view
    }

    /// Schedule a callback to run after the next [`ModelEvent::AfterBlockCompleted`] received.
    fn on_next_block_completed<F>(&mut self, callback: F)
    where
        F: FnOnce(&mut Self, &mut ViewContext<Self>) + 'static,
    {
        self.block_completed_callbacks.push(Box::new(callback));
    }

    fn set_pending_cloud_mode_start_callback(
        &mut self,
        callback: TerminalViewCallback,
        ctx: &mut ViewContext<Self>,
    ) {
        self.clear_pending_cloud_mode_start_callback();
        self.pending_cloud_mode_start_callback = Some(callback);

        self.pending_cloud_mode_start_abort_handle = Some(ctx.spawn_abortable(
            // Reuse the same timeout as agent-view confirmation prompts so a pending cloud-mode
            // start cannot outlive the user-visible confirmation window semantics.
            Timer::after(ENTER_OR_EXIT_CONFIRMATION_WINDOW),
            |me, _, _ctx| {
                me.pending_cloud_mode_start_callback = None;
                me.pending_cloud_mode_start_abort_handle = None;
            },
            |_, _| (),
        ));
    }

    fn clear_pending_cloud_mode_start_callback(&mut self) {
        if let Some(handle) = self.pending_cloud_mode_start_abort_handle.take() {
            handle.abort();
        }
        self.pending_cloud_mode_start_callback = None;
    }

    fn maybe_run_pending_cloud_mode_start_callback(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(callback) = self.pending_cloud_mode_start_callback.take() else {
            return;
        };

        if let Some(handle) = self.pending_cloud_mode_start_abort_handle.take() {
            handle.abort();
        }

        callback(self, ctx);
    }

    /// If the active conversation is a child agent, navigate to the parent
    /// and return `true`; otherwise return `false` so the caller can run
    /// the normal exit-agent-view flow. Cross-tab and swap-target cases
    /// are handled by the workspace's focus path; falls back to emitting
    /// a swap event when the parent has no canonical owner. Runs before
    /// any can-exit gating so long-running children can still navigate back.
    fn try_navigate_to_parent_conversation(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        if !FeatureFlag::AgentView.is_enabled() {
            return false;
        }
        let active_conv_id = self
            .agent_view_controller
            .as_ref(ctx)
            .agent_view_state()
            .active_conversation_id();
        let Some(active_conv_id) = active_conv_id else {
            return false;
        };
        let history = BlocklistAIHistoryModel::as_ref(ctx);
        let parent_id = history
            .conversation(&active_conv_id)
            .and_then(|c| c.parent_conversation_id());
        let Some(parent_id) = parent_id else {
            return false;
        };
        let parent_terminal_view_id = history.terminal_view_id_for_conversation(&parent_id);

        if let Some(parent_terminal_view_id) = parent_terminal_view_id {
            // Defer so it runs after in-flight event handling completes.
            ctx.dispatch_typed_action_deferred(WorkspaceAction::FocusTerminalViewInWorkspace {
                terminal_view_id: parent_terminal_view_id,
            });
        } else {
            ctx.emit(Event::SwapPaneToConversation {
                conversation_id: parent_id,
            });
        }
        true
    }

    /// Exits the active agent, either:
    /// * Exiting agent view for the selected conversation
    /// * Popping the current view off the navigation stack (for nested cloud mode agents)
    /// Root cloud-mode panes (stack depth ≤ 1) are a no-op — there is nowhere to return to.
    fn exit_agent_view(&mut self, ctx: &mut ViewContext<Self>) {
        // For nested ambient agent sessions (cloud mode), pop from pane stack.
        // Root cloud-mode panes have no parent terminal to return to, so escape
        // is a no-op to avoid leaving the app in a borked state.
        if self.is_ambient_agent_session(ctx) {
            if let Some(pane_stack) = self
                .pane_stack
                .as_ref()
                .and_then(|h| h.upgrade(ctx))
                .filter(|stack| stack.as_ref(ctx).depth() > 1)
            {
                pane_stack.update(ctx, |stack, ctx| {
                    stack.pop(ctx);
                });
            }
        } else {
            self.agent_view_controller.update(ctx, |controller, ctx| {
                controller.exit_agent_view(ctx);
            });
        }
    }

    /// Schedule a callback to run after the next
    /// [`BlocklistAIControllerEvent::FinishedReceivingOutput`] received, regardless of whether the
    /// conversation completed successfully, was cancelled, or encountered an error.
    /// The callback receives the `FinishReason` to allow different handling based on how the
    /// conversation ended.
    pub fn on_next_conversation_finished<F>(&mut self, callback: F)
    where
        F: FnOnce(&mut Self, FinishReason, &mut ViewContext<Self>) + 'static,
    {
        self.conversation_completed_callbacks
            .push(Box::new(callback));
    }

    fn handle_finished_conversation(
        &mut self,
        conversation_id: AIConversationId,
        finish_reason: FinishReason,
        ctx: &mut ViewContext<Self>,
    ) {
        let queued_prompt = self.queued_prompt_callback.take();
        let callbacks = self
            .conversation_completed_callbacks
            .drain(..)
            .collect_vec();
        for callback in callbacks {
            callback(self, finish_reason, ctx);
        }
        if let Some(callback) = queued_prompt {
            callback(self, finish_reason, ctx);
        }
        self.drain_queued_prompts(conversation_id, finish_reason, ctx);
    }

    #[cfg(feature = "local_fs")]
    fn handle_git_repo_status_event(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(deferred) = self.deferred_code_review_open.take() {
            self.toggle_code_review_pane(
                deferred.git_delta_preference,
                CodeReviewPaneEntrypoint::Other,
                None,
                deferred.focus_new_pane,
                ctx,
            );
        }
        self.refresh_pane_header(ctx);
        ctx.emit(Event::TerminalViewStateChanged);
        ctx.notify();
    }

    /// Drop the per-repo git status subscription without clearing the input's
    /// repo path. Use this when unsubscribing because the subscription is no
    /// longer needed (e.g. the git chip was removed) but the user is still in
    /// the same repository.
    #[cfg(feature = "local_fs")]
    fn clear_git_repo_status_subscription(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(handle) = self.git_repo_status.take() {
            let terminal_view_id = self.view_id;
            handle.update(ctx, |model, ctx| {
                model.set_pr_info_consumer(terminal_view_id, false, ctx);
            });
            ctx.unsubscribe_to_model(&handle);
        }
        self.deferred_code_review_open = None;

        self.current_prompt.update(ctx, |prompt_type, ctx| {
            if let PromptType::Dynamic { prompt } = prompt_type {
                prompt.update(ctx, |current_prompt, ctx| {
                    current_prompt.set_git_repo_status(None, ctx);
                });
            }
        });
    }

    /// Fully clear the per-repo git status handle, including the input's repo
    /// path. Use this when navigating out of a git repository.
    #[cfg(feature = "local_fs")]
    fn clear_git_repo_status(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_git_repo_status_subscription(ctx);
        self.input.update(ctx, |input, ctx| {
            input.update_repo_path(None, ctx);
        });
    }

    /// Helper to read metadata from the per-repo sub-model.
    #[cfg(feature = "local_fs")]
    fn git_status_metadata<'a>(&'a self, ctx: &'a AppContext) -> Option<&'a GitStatusMetadata> {
        self.git_repo_status
            .as_ref()
            .and_then(|h| h.as_ref(ctx).metadata())
    }

    /// Returns whether this terminal view should subscribe to git status updates.
    /// We subscribe when:
    /// 1. Agent mode is active and its chip list includes `GitDiffStats` or `GithubPullRequest`, or
    /// 2. Terminal mode with the Warp prompt enabled and the git stats chip
    ///    configured.
    #[cfg(feature = "local_fs")]
    fn should_subscribe_to_git_status(&self, ctx: &AppContext) -> bool {
        let uses_git_status = |chips: Vec<ContextChipKind>| {
            chips.iter().any(|chip| {
                matches!(
                    chip,
                    ContextChipKind::GitDiffStats | ContextChipKind::GithubPullRequest
                )
            })
        };

        // Agent view: subscribe when the configured agent footer includes
        // git stats or PR info.
        if self.agent_view_controller.as_ref(ctx).is_active() {
            return uses_git_status(
                SessionSettings::as_ref(ctx)
                    .agent_footer_chip_selection
                    .all_chips(),
            );
        }
        // CLI-agent footer: subscribe only while a CLI-agent session is active,
        // so normal terminal panes do not subscribe just because of CLI footer defaults.
        if self.has_active_cli_agent_session(ctx)
            && uses_git_status(
                SessionSettings::as_ref(ctx)
                    .cli_agent_footer_chip_selection
                    .all_chips(),
            )
        {
            return true;
        }

        // Terminal prompt path: the Warp prompt is active when honor_ps1 is
        // off, or when UDI overrides PS1. The prompt must include a chip backed
        // by git status.
        let is_using_warp_prompt = !*SessionSettings::as_ref(ctx).honor_ps1
            || InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx);
        if is_using_warp_prompt && Self::should_retry_default_pr_chip_validation(ctx) {
            return true;
        }
        is_using_warp_prompt && uses_git_status(Prompt::as_ref(ctx).chip_kinds())
    }

    /// Whether the terminal's prompt/footer chips need PR info.
    #[cfg(feature = "local_fs")]
    fn needs_pr_info(&self, ctx: &AppContext) -> bool {
        if self.agent_view_controller.as_ref(ctx).is_active() {
            return SessionSettings::as_ref(ctx)
                .agent_footer_chip_selection
                .all_chips()
                .contains(&ContextChipKind::GithubPullRequest);
        }
        if self.has_active_cli_agent_session(ctx)
            && SessionSettings::as_ref(ctx)
                .cli_agent_footer_chip_selection
                .all_chips()
                .contains(&ContextChipKind::GithubPullRequest)
        {
            return true;
        }

        let is_using_warp_prompt = !*SessionSettings::as_ref(ctx).honor_ps1
            || InputSettings::as_ref(ctx).is_universal_developer_input_enabled(ctx);
        is_using_warp_prompt
            && (Self::should_retry_default_pr_chip_validation(ctx)
                || Prompt::as_ref(ctx)
                    .chip_kinds()
                    .contains(&ContextChipKind::GithubPullRequest))
    }

    #[cfg(feature = "local_fs")]
    fn should_retry_default_pr_chip_validation(ctx: &AppContext) -> bool {
        let settings = SessionSettings::as_ref(ctx);
        FeatureFlag::GithubPrPromptChip.is_enabled()
            && settings.github_pr_chip_default_validation.is_suppressed()
            && matches!(
                *settings.saved_prompt,
                crate::context_chips::prompt::PromptSelection::Default
            )
    }

    /// Refresh the terminal's own `pr_info_consumer` registration on the
    /// current git status handle. Each consumer manages its own slot; this
    /// only toggles the terminal's slot.
    #[cfg(feature = "local_fs")]
    fn sync_pr_info_consumer_for_current_subscription(&self, ctx: &mut ViewContext<Self>) {
        let Some(handle) = &self.git_repo_status else {
            return;
        };
        let terminal_view_id = self.view_id;
        let needs_pr_info = self.needs_pr_info(ctx);
        handle.update(ctx, |model, ctx| {
            model.set_pr_info_consumer(terminal_view_id, needs_pr_info, ctx);
        });
    }

    /// Triggers a PR info refresh after a `gh`/`gt` command completes.
    ///
    /// These commands don't touch `.git/` so the filesystem watcher won't
    /// catch them; we refresh explicitly while an active PR-info consumer is
    /// registered for this terminal.
    #[cfg(feature = "local_fs")]
    fn refresh_pr_info_after_gh_or_gt_command(&mut self, ctx: &mut ViewContext<Self>) {
        // Ensure we have a subscription to the per-repo status model.
        // `should_subscribe_to_git_status` already returns true while
        // suppression is active so the default chip can recover, so this
        // is a no-op when already subscribed and creates a fresh
        // subscription when one is needed.
        self.update_git_status_subscription(ctx);

        let Some(handle) = self.git_repo_status.clone() else {
            return;
        };
        handle.update(ctx, |model, ctx| {
            model.refresh_pr_info(ctx);
        });
    }

    /// No-op when the `local_fs` feature is disabled – git status is not
    /// available so there is nothing to subscribe to.
    #[cfg(not(feature = "local_fs"))]
    fn update_git_status_subscription(&mut self, _ctx: &mut ViewContext<Self>) {}

    /// Re-evaluate whether this terminal view should be subscribed to git
    /// status updates and subscribe/unsubscribe accordingly.
    #[cfg(feature = "local_fs")]
    fn update_git_status_subscription(&mut self, ctx: &mut ViewContext<Self>) {
        let should_subscribe = self.should_subscribe_to_git_status(ctx);
        if should_subscribe {
            // Subscribe if we have a repo path but no active subscription.
            if self.git_repo_status.is_some() {
                self.sync_pr_info_consumer_for_current_subscription(ctx);
            } else if let Some(repo_path) = self.current_local_repo_path().map(Path::to_path_buf) {
                let result = GitStatusUpdateModel::handle(ctx)
                    .update(ctx, |model, ctx| model.subscribe(&repo_path, ctx));
                match result {
                    Ok(handle) => {
                        ctx.subscribe_to_model(&handle, |me, _, _, ctx| {
                            me.handle_git_repo_status_event(ctx);
                        });
                        let weak_for_prompt = handle.downgrade();
                        self.git_repo_status = Some(handle);
                        self.current_prompt.update(ctx, |prompt_type, ctx| {
                            if let PromptType::Dynamic { prompt } = prompt_type {
                                prompt.update(ctx, |current_prompt, ctx| {
                                    current_prompt.set_git_repo_status(Some(weak_for_prompt), ctx);
                                });
                            }
                        });
                        // Register the terminal as a `pr_info` consumer if its
                        // prompt/footer needs PR info; the per-repo model only
                        // fetches PR info while at least one consumer is
                        // registered.
                        self.sync_pr_info_consumer_for_current_subscription(ctx);
                    }
                    Err(err) => {
                        log::warn!("GitStatusUpdateModel subscribe failed: {err}");
                    }
                }
            }
        } else if self.git_repo_status.is_some() {
            self.clear_git_repo_status_subscription(ctx);
        }
    }

    #[cfg(feature = "local_fs")]
    fn handle_attach_diffset_context(&mut self, diff_mode: DiffMode, ctx: &mut ViewContext<Self>) {
        let Some(repo_path) = self.current_local_repo_path().map(Path::to_path_buf) else {
            return;
        };

        // Get branch information from the per-repo sub-model.
        let metadata = self.git_status_metadata(ctx);
        let current_branch = metadata.map(|m| m.current_branch_name.clone());
        let current = current_branch.map(CurrentHead::BranchName);

        let base = match &diff_mode {
            DiffMode::Head => DiffBase::UncommittedChanges,
            DiffMode::MainBranch => metadata
                .map(|m| DiffBase::BranchName(m.main_branch_name.clone()))
                .unwrap_or(DiffBase::UncommittedChanges),
            DiffMode::OtherBranch(branch_name) => DiffBase::BranchName(branch_name.clone()),
        };

        // Create attachment reference and key using the shared function
        let main_branch_name = metadata.map(|m| m.main_branch_name.clone());
        let (attachment_reference, diff_set_key) = create_attachment_reference_and_key(
            &DiffSetScope::All,
            &diff_mode,
            main_branch_name.as_deref(),
        );

        // Insert the reference into the terminal input immediately
        self.input.update(ctx, |input, ctx| {
            // Remove the @-trigger text (e.g. "@uncom") that was used to open the context menu.
            input.replace_at_symbol_with_text(&attachment_reference, ctx);
            input.ensure_agent_mode_for_ai_features(
                true,
                Some(InputTypeAutoDetectionSource::AttachmentForcedAi),
                ctx,
            );
        });

        // Load the diff data asynchronously and complete the attachment when done
        let ai_context_model = self.ai_context_model.clone();
        let diff_mode_clone = diff_mode.clone();
        let repo_path_clone = repo_path.clone();
        let future = async move {
            LocalDiffStateModel::load_diff_data_for_mode(diff_mode_clone, repo_path_clone).await
        };

        ctx.spawn(future, move |_me, git_diff_data_opt, ctx| {
            let Some(git_diff_data) = git_diff_data_opt else {
                return;
            };

            let file_diffs = convert_file_diffs_to_diffset_hunks(git_diff_data.files.iter());

            register_diffset_attachment(
                &ai_context_model,
                diff_set_key,
                file_diffs,
                current,
                base,
                ctx,
            );
        });
    }

    #[cfg(any(test, feature = "integration_tests"))]
    pub fn sessions<'a, A: warpui::ModelAsRef>(&self, ctx: &'a A) -> &'a Sessions {
        self.sessions.as_ref(ctx)
    }
    #[cfg(test)]
    pub fn model_event_dispatcher(&self) -> &ModelHandle<ModelEventDispatcher> {
        &self.model_events_handle
    }
    #[cfg(test)]
    pub(crate) fn is_initial_conversation_details_panel_auto_open_suppressed_for_test(
        &self,
    ) -> bool {
        matches!(
            self.conversation_details_panel_auto_open_policy,
            ConversationDetailsPanelAutoOpenPolicy::DefaultClosed
        )
    }
    #[cfg(windows)]
    fn ctrl_c_internal(
        &mut self,
        has_copiable_block_selection: bool,
        has_block_list_selection: bool,
        has_alt_screen_selection: bool,
        is_long_running: bool,
        is_agent_in_control_of_command: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if has_block_list_selection {
            self.copy(ctx);
            self.clear_selections_when_shell_mode_without_focusing_input(ctx);
            return;
        } else if has_alt_screen_selection {
            self.copy(ctx);
            self.model.lock().alt_screen_mut().clear_selection();
            return;
        } else if has_copiable_block_selection {
            // If there are blocks selected, we want to copy them but
            // not prevent the normal ctrl-c behaviour.
            self.copy(ctx);
            self.clear_selections_when_shell_mode_without_focusing_input(ctx);
        }

        self.ctrl_c_to_active_block(is_long_running, is_agent_in_control_of_command, ctx);
    }
    #[cfg(not(windows))]
    fn ctrl_c_internal(
        &mut self,
        has_copiable_block_selection: bool,
        has_block_list_selection: bool,
        has_alt_screen_selection: bool,
        is_long_running: bool,
        is_agent_in_control_of_command: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if has_block_list_selection || has_copiable_block_selection {
            self.clear_selections_when_shell_mode_without_focusing_input(ctx);
        } else if has_alt_screen_selection {
            self.model.lock().alt_screen_mut().clear_selection();
        }
        self.ctrl_c_to_active_block(is_long_running, is_agent_in_control_of_command, ctx);
    }
    #[cfg(feature = "integration_tests")]
    pub fn is_secret_tooltip_open(&self) -> bool {
        self.open_secret_tool_tip.is_some()
    }
    #[cfg(feature = "local_fs")]
    fn insert_agent_mode_setup_speedbump_banner(
        &mut self,
        repo_path: PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        // Create new inline banner
        let banner_id = self.inline_banners_state.next_banner_id();
        let banner_state = AgentModeSetupSpeedbumpBannerState::new(banner_id, repo_path.clone());

        // Insert the banner into the block list
        self.model
            .lock()
            .block_list_mut()
            .append_inline_banner_with_custom_height(
                InlineBannerItem::new(banner_id, InlineBannerType::AgentModeSetup),
                4.0,
            );

        // Store the banner state
        self.inline_banners_state.agent_setup_speedbump_banner = Some(banner_state);

        // Track that this banner has been shown for this repo
        // so it won't be shown again
        self.mark_agent_init_callout_as_shown_for_directory(&repo_path, ctx);

        ctx.notify();
    }
    #[cfg(feature = "local_fs")]
    fn insert_codebase_index_speedbump_banner(
        &mut self,
        repo_path: PathBuf,
        show_is_indexing: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        // Create new inline banner
        let banner_id = self.inline_banners_state.next_banner_id();
        let mut banner_state = CodebaseIndexSpeedbumpBannerState::new(banner_id, repo_path);
        if show_is_indexing {
            banner_state.show_indexing_banner(); // Set to indexing state
        }

        // Insert the banner into the block list
        self.model
            .lock()
            .block_list_mut()
            .append_inline_banner_with_custom_height(
                InlineBannerItem::new(banner_id, InlineBannerType::CodebaseIndexSpeedbump),
                4.0,
            );

        // Store the banner state
        self.inline_banners_state.codebase_index_speedbump_banner = Some(banner_state);

        ctx.notify();
    }
    #[cfg(feature = "local_fs")]
    fn remove_codebase_index_speedbump_banner(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(banner_state) = self
            .inline_banners_state
            .codebase_index_speedbump_banner
            .take()
        {
            self.model
                .lock()
                .block_list_mut()
                .remove_inline_banner(banner_state.id);
            ctx.notify();
        }
    }
    #[cfg(feature = "local_fs")]
    fn remove_agent_setup_speedbump_banner(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(banner_state) = self
            .inline_banners_state
            .agent_setup_speedbump_banner
            .take()
        {
            self.model
                .lock()
                .block_list_mut()
                .remove_inline_banner(banner_state.id);
            ctx.notify();
        }
    }
    #[cfg(not(feature = "local_fs"))]
    fn remove_agent_setup_speedbump_banner(&mut self, _ctx: &mut ViewContext<Self>) {
        // No-op when local filesystem is unavailable.
    }
    #[cfg(feature = "integration_tests")]
    pub fn current_prompt(&self) -> ModelHandle<PromptType> {
        self.current_prompt.clone()
    }
    #[cfg(feature = "local_fs")]
    fn update_repo_banner_state(&mut self, directory: PathBuf, ctx: &mut ViewContext<Self>) {
        self.update_agent_mode_setup_speedbump_banner(directory, ctx);
    }
    #[cfg(not(feature = "local_fs"))]
    fn update_repo_banner_state(&mut self, _directory: PathBuf, _ctx: &mut ViewContext<Self>) {
        // Repo setup is not supported without a local filesystem.
    }
    #[cfg(feature = "local_fs")]
    fn update_agent_mode_setup_speedbump_banner(
        &mut self,
        directory: PathBuf,
        ctx: &mut ViewContext<Self>,
    ) {
        let should_insert_banner = self.should_show_agent_mode_setup_for_directory(&directory, ctx)
            && !FeatureFlag::AgentView.is_enabled();

        if !should_insert_banner {
            self.remove_agent_setup_speedbump_banner(ctx);
            return;
        }

        if let Some(banner_state) = &self.inline_banners_state.agent_setup_speedbump_banner {
            if banner_state.repo_path != directory {
                // If the banner is showing for a different repo, remove it, and insert it for the new repo.
                self.remove_agent_setup_speedbump_banner(ctx);
                self.insert_agent_mode_setup_speedbump_banner(directory, ctx);
            }
        } else {
            // If no banner exists, insert it.
            self.insert_agent_mode_setup_speedbump_banner(directory, ctx);
        }
    }
    #[cfg(feature = "local_fs")]
    fn should_show_agent_mode_setup_for_directory(
        &self,
        directory: &Path,
        ctx: &AppContext,
    ) -> bool {
        let already_shown = AISettings::as_ref(ctx)
            .agent_mode_setup_banner_shown_for_repo_paths
            .value()
            .iter()
            .any(|shown_path| shown_path == directory);
        let is_repo = DetectedRepositories::as_ref(ctx)
            .get_root_for_path(&LocalOrRemotePath::Local(directory.to_path_buf()))
            .is_some();
        let is_any_ai_enabled =
            FeatureFlag::AgentMode.is_enabled() && AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        // Check if the current session is remote - don't show setup in remote sessions.
        let is_remote_session = !self.active_session_is_local(ctx).unwrap_or(false);

        // Condition for showing setup:
        // 1) Has not already shown
        // 2) AI is enabled
        // 3) Directory is in an active repo
        // 4) There is no in-progress AI conversation (we don't want setup to show up mid conversation flow)
        // 5) Session is not remote
        // 6) There are available steps to show
        !already_shown
            && is_any_ai_enabled
            && is_repo
            && self.active_ai_block(ctx).is_none()
            && !is_remote_session
            && InitProjectModel::should_have_available_steps(directory, ctx)
    }
    #[cfg(not(feature = "local_fs"))]
    fn should_show_agent_mode_setup_for_directory(
        &self,
        _directory: &Path,
        _ctx: &AppContext,
    ) -> bool {
        false
    }
    #[cfg(feature = "local_tty")]
    fn resize_alt_screen_redundantly(&mut self, ctx: &mut ViewContext<Self>) {
        use futures_lite::StreamExt;

        // Resize twice, half a second apart.
        ctx.spawn_stream_local(
            async_io::Timer::interval(Duration::from_millis(500)).take(2),
            |view, _, ctx| {
                let model = view.model.lock();

                // If the alt-screen was exited since the timer expired,
                // there's nothing to do.
                if !model.is_alt_screen_active() {
                    return;
                }

                let correct_size_info = *view.size_info;
                let active_command = model
                    .block_list()
                    .active_block()
                    .top_level_command(view.sessions.as_ref(ctx));

                // Drop the lock since the resize methods will take an explicit lock.
                drop(model);

                // This is a workaround for alt-screen programs that _cache_ resizes during init
                // but don't actually redraw the contents. For example, we've seen this happen with
                // certain emacs setups. So we fake a winsize before immediately correcting it to
                // invalidate that cache.
                if active_command
                    .is_some_and(|cmd| ALT_SCREEN_APPS_WITH_RESIZE_PROBLEMS.contains(cmd.as_str()))
                {
                    let mut wrong_size_info = *view.size_info;
                    wrong_size_info.pane_width_px += 1.;
                    view.resize_internal(
                        SizeUpdateBuilder::for_refresh(wrong_size_info).build(view, ctx),
                        ctx,
                    );
                }

                // Send the resize as a refresh to force a size update.
                view.resize_internal(
                    SizeUpdateBuilder::for_refresh(correct_size_info).build(view, ctx),
                    ctx,
                );
            },
            |_, _| {},
        );
    }

    async fn fetch_command_corrections(
        block: UserBlockCompleted,
        session: Option<Arc<Session>>,
        history_commands: Vec<HistoryEntry>,
    ) -> Vec<Correction> {
        // Create the command
        let (input, output, exit_code, working_dir) = (
            block.command.as_str(),
            block.output_truncated.as_str(),
            block.serialized_block.exit_code,
            block.serialized_block.pwd.as_ref(),
        );

        let mut command = Command::new(input, output, exit_code.into());
        if let Some(working_dir) = working_dir {
            command = command.set_working_dir(working_dir);
        }

        // Create the session metadata
        // TODO: we need to figure out how to avoid re-creating this
        // for every single invocation of correct_command.
        let mut session_metadata = SessionMetadata::new();

        session_metadata.set_history(history_commands.iter().filter_map(|s| {
            Some(HistoryItem::new(
                s.command.as_str(),
                s.exit_code?,
                s.pwd.as_ref()?.as_str(),
            ))
        }));

        let mut git_branches = None;
        if let Some(session) = &session {
            session_metadata.set_session_type(session.session_type().clone().into());
            let shell = session.shell();
            session_metadata.set_shell(shell.shell_type().into(), shell.version().as_deref());
            session_metadata.set_aliases(session.alias_names());
            session_metadata.set_executables(session.executable_names());
            session_metadata.set_functions(session.function_names());
            session_metadata.set_builtins(session.builtin_names());
            session_metadata.set_platform_type(session.host_info().platform_type());
            if let Some(working_dir) = working_dir {
                git_branches = Some(
                    session
                        .git_branches_for_command_corrections(working_dir)
                        .await,
                );
            }
        }
        session_metadata.set_git_branches(git_branches.iter().flatten().map(|s| s.as_str()));

        // https://github.com/warpdotdev/command-corrections/blob/df7848d4fb3da7883623e959889a296a07d88053/src/rules/cd/mod.rs#L31-L36
        // We don't currently support dynamic rules over SSH, so we should not attempt to correct commands if
        // inside ssh session.
        let is_ssh_command = SshWarpifyCommand::matches(input).is_some();
        if is_ssh_command {
            return vec![];
        }
        if FeatureFlag::CommandCorrectionsHistoryRule.is_enabled() {
            correct_command(command, &session_metadata, std::iter::empty())
        } else {
            correct_command(
                command,
                &session_metadata,
                DEFAULT_IGNORED_RULES_FOR_COMMAND_CORRECTIONS.into_iter(),
            )
        }
    }

    fn write_init_subshell_bytes_to_pty(
        &mut self,
        shell_type: Option<ShellType>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.clear_line_editor_and_write_to_pty(
            init_subshell_command(shell_type, &self.env_vars, ctx).into_bytes(),
            ctx,
        );
        self.write_to_pty(vec![escape_sequences::C0::CR], ctx);
    }

    /// If a command correction exists, generate the command correction banner.
    fn after_command_correction_generation(
        &mut self,
        corrections: Vec<Correction>,
        ctx: &mut ViewContext<TerminalView>,
    ) {
        if let Some(correction) = corrections.into_iter().next() {
            let rule = correction.rule_applied;

            if AISettings::as_ref(ctx).is_intelligent_autosuggestions_enabled(ctx)
                && UserWorkspaces::as_ref(ctx).is_next_command_enabled()
                && COMMAND_CORRECTIONS_PREFERRED_DENYLIST.contains(rule.to_str())
            {
                // Defer to Next Command if the rule is in the denylist.
                return;
            }

            // Set the autosuggestion only if the input is still empty
            self.input.update(ctx, |input, ctx| {
                if input.buffer_text(ctx).is_empty() {
                    input.set_autosuggestion(
                        correction.command.as_str(),
                        AutosuggestionType::Command {
                            was_intelligent_autosuggestion: false,
                        },
                        ctx,
                    );
                }
            });

            let a11y_content = AccessibilityContent::new(
                format!("Suggested corrected command: {}", correction.command),
                "Press right arrow to insert or keep editing to ignore",
                WarpA11yRole::HelpRole,
            );
            ctx.emit_a11y_content(a11y_content);

            self.most_recent_command_correction = Some(correction);

            ctx.notify();

            send_telemetry_from_ctx!(
                TelemetryEvent::CommandCorrection {
                    event: CommandCorrectionEvent::Proposed {
                        rule: rule.to_str()
                    }
                },
                ctx
            );
        }
    }

    fn clear_prompt_suggestions(&mut self, ctx: &mut ViewContext<Self>) {
        if self
            .inline_banners_state
            .prompt_suggestions_banner
            .take()
            .is_some()
        {
            self.input.update(ctx, |input, ctx| {
                input.set_prompt_suggestions_banner_state(None, ctx);
                input.notify_and_notify_children(ctx);
            });
        }
        if let Some(ai_block) = self.last_ai_block() {
            ai_block.update(ctx, |ai_block, ctx| {
                ai_block.ignore_passive_actions(ctx);
            });
        };
    }

    fn update_input_prompt_suggestions_banner_state(&mut self, ctx: &mut ViewContext<Self>) {
        for rich_content in &self.rich_content_views {
            if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                // If the passive code gen fails, show the prompt suggestion banner as a fallback
                if ai_metadata
                    .ai_block_handle
                    .as_ref(ctx)
                    .is_passive_conversation(ctx)
                    && matches!(
                        ai_metadata.ai_block_handle.as_ref(ctx).status(ctx),
                        AIBlockOutputStatus::Failed { .. }
                    )
                {
                    // Try to update the state of the prompt suggestions banner
                    self.input.update(ctx, |input, ctx| {
                        input.maybe_set_prompt_suggestions_banner_state_should_hide(false);
                        input.notify_and_notify_children(ctx);
                    });

                    break;
                }
            }
        }
    }

    /// Removes hidden AI blocks for passive requests from the sumtree.
    ///
    /// Hidden AI blocks are only generated when generating passive codegen suggestions after a
    /// compiler error.
    fn drop_hidden_passive_ai_blocks(&mut self, ctx: &mut ViewContext<Self>) {
        let mut ai_block_ids_to_remove = vec![];
        self.rich_content_views.retain(|rich_content| {
            if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                let is_hidden = ai_metadata.ai_block_handle.read(ctx, |ai_block, ctx| {
                    ai_block.is_hidden(ctx) && ai_block.is_passive_conversation(ctx)
                });
                if is_hidden {
                    ai_block_ids_to_remove.push(ai_metadata.ai_block_handle.id());
                }
                !is_hidden
            } else {
                true
            }
        });

        for view_id in ai_block_ids_to_remove {
            self.model
                .lock()
                .block_list_mut()
                .remove_rich_content(view_id);
        }

        self.update_input_prompt_suggestions_banner_state(ctx);
        ctx.notify();
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn remove_plugin_instructions_block(
        &mut self,
        block_handle: ViewHandle<plugin_instructions_block::PluginInstructionsBlock>,
        ctx: &mut ViewContext<Self>,
    ) {
        let block_id = block_handle.id();
        self.rich_content_views
            .retain(|rich_content| rich_content.view_id() != block_id);
        self.model
            .lock()
            .block_list_mut()
            .remove_rich_content(block_id);
        ctx.notify();
    }

    /// Removes AI blocks from `rich_content_views` that match the given conversation and exchange IDs.
    /// This handles cleanup of the block, removal from the block list model, and notifying the
    /// new last AI block in the conversation so it re-renders with the footer.
    fn remove_ai_blocks_for_exchanges(
        &mut self,
        conversation_id: &AIConversationId,
        exchange_ids: &HashSet<AIAgentExchangeId>,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut blocks_to_remove: Vec<(EntityId, ViewHandle<AIBlock>)> = vec![];
        self.rich_content_views.retain(|rich_content| {
            if let Some(ai_metadata) = rich_content.ai_block_metadata() {
                if ai_metadata.conversation_id == *conversation_id
                    && exchange_ids.contains(&ai_metadata.exchange_id)
                {
                    blocks_to_remove.push((
                        ai_metadata.ai_block_handle.id(),
                        ai_metadata.ai_block_handle.clone(),
                    ));
                    return false;
                }
            }
            true
        });

        // Close any open usage footers on blocks being removed to prevent them becoming orphaned
        for (view_id, handle) in &blocks_to_remove {
            if self.usage_footer_view_ids.contains_key(view_id) {
                handle.update(ctx, |block, ctx| {
                    block.handle_action(&AIBlockAction::ToggleIsUsageFooterExpanded, ctx);
                });
            }
        }

        blocks_to_remove.into_iter().for_each(|(view_id, handle)| {
            handle.update(ctx, |block, ctx| {
                block.cleanup_block(ctx);
            });
            self.model
                .lock()
                .block_list_mut()
                .remove_rich_content(view_id);
        });

        // Notify the new last AI block so it re-renders with the footer
        if let Some(new_last_block) = self.rich_content_views.iter().rev().find_map(|rc| {
            let ai_metadata = rc.ai_block_metadata()?;
            if ai_metadata.conversation_id == *conversation_id {
                return Some(ai_metadata.ai_block_handle.clone());
            }
            None
        }) {
            new_last_block.update(ctx, |_, ctx| ctx.notify());
        }

        // Update scroll position to ensure we don't have blank space
        self.update_scroll_position_locking(ScrollPositionUpdate::AfterEnd, ctx);
    }

    fn handle_maa_passive_suggestions_event(
        &mut self,
        _: ModelHandle<MaaPassiveSuggestionsModel>,
        event: &MaaPassiveSuggestionsEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            MaaPassiveSuggestionsEvent::NewPromptSuggestion {
                prompt,
                label,
                request_duration_ms,
                trigger,
                conversation_id,
                server_request_token,
            } => {
                self.on_maa_prompt_suggestion_generated(
                    prompt,
                    &label.clone(),
                    *request_duration_ms,
                    trigger.clone(),
                    *conversation_id,
                    server_request_token.clone(),
                    ctx,
                );
            }
            MaaPassiveSuggestionsEvent::NewCodeDiffSuggestion {
                diffs,
                edit_format_kind,
                title,
                original_edits,
                conversation_id,
                request_duration_ms,
                trigger,
                server_request_token,
            } => {
                self.on_maa_code_diff_generated(
                    diffs.clone(),
                    *edit_format_kind,
                    title.clone(),
                    original_edits.clone(),
                    *conversation_id,
                    *request_duration_ms,
                    trigger.clone(),
                    server_request_token.clone(),
                    ctx,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn on_maa_prompt_suggestion_generated(
        &mut self,
        prompt: &str,
        label: &Option<String>,
        request_duration_ms: u64,
        trigger: Option<PassiveSuggestionTrigger>,
        conversation_id: Option<AIConversationId>,
        server_request_token: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if prompt.is_empty() {
            return;
        }

        self.clear_prompt_suggestions(ctx);
        let block_id = trigger.as_ref().and_then(|t| t.block_id());
        let suggestion_id = Uuid::new_v4().to_string();
        let banner_id = self.inline_banners_state.next_banner_id();
        let banner_state = PromptSuggestionBannerState {
            banner_id,
            prompt_suggestion: PromptSuggestion {
                id: suggestion_id.clone(),
                label: label.clone(),
                prompt: prompt.to_string(),
                coding_query_context: None,
                static_prompt_suggestion_name: None,
                should_start_new_conversation: false,
            },
            accept_button_mouse_state: Default::default(),
            llm_warning_learn_more_hyperlink: Default::default(),
            should_hide: false,
            trigger,
            conversation_id,
            server_request_token: server_request_token.clone(),
        };

        self.inline_banners_state.prompt_suggestions_banner = Some(banner_state.clone());
        self.input.update(ctx, |input, ctx| {
            input.set_prompt_suggestions_banner_state(Some(banner_state), ctx);
            input.notify_and_notify_children(ctx);
        });

        send_telemetry_from_ctx!(
            TelemetryEvent::PromptSuggestionShown {
                id: suggestion_id,
                request_duration_ms,
                block_id: block_id.map(|b| b.to_string()),
                view: self.prompt_suggestion_view_type(ctx),
                server_request_token,
            },
            ctx
        );

        ctx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn on_maa_code_diff_generated(
        &mut self,
        diffs: Vec<FileDiff>,
        edit_format_kind: RequestFileEditsFormatKind,
        title: Option<String>,
        original_edits: Vec<PassiveCodeDiffEntry>,
        conversation_id: Option<AIConversationId>,
        request_duration_ms: u64,
        trigger: PassiveSuggestionTrigger,
        server_request_token: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        let action_id = AIAgentActionId::from(uuid::Uuid::new_v4().to_string());
        use crate::ai::agent::AIIdentifiers;
        use crate::ai::blocklist::inline_action::code_diff_view::CodeDiffViewEvent;

        let identifiers = AIIdentifiers::default();
        let title_for_result = title.clone();

        let session_platform = self
            .active_session
            .as_ref(ctx)
            .shell_launch_data(ctx)
            .map(Into::into);

        let diff_view = ctx.add_typed_action_view(|ctx| {
            CodeDiffView::new_passive(
                &action_id,
                title,
                identifiers,
                edit_format_kind,
                false,
                session_platform,
                ctx,
            )
        });

        diff_view.update(ctx, |view, ctx| {
            view.set_candidate_diffs(diffs, ctx);
        });

        let wrapper_view = {
            let diff_view_for_wrapper = diff_view.clone();
            ctx.add_view(move |_ctx| inline_banner::PassiveCodeDiff {
                diff_view: diff_view_for_wrapper,
            })
        };

        let trigger_block_id = match &trigger {
            PassiveSuggestionTrigger::ShellCommandCompleted(trigger) => {
                Some(trigger.executed_shell_command.id.clone())
            }
            _ => None,
        };
        // Capture the string form for telemetry before `trigger_block_id` is
        // moved into the subscribe_to_view closure below.
        let trigger_block_id_str = trigger_block_id.as_ref().map(|id| id.to_string());

        let wrapper_view_id = wrapper_view.id();
        ctx.subscribe_to_view(&diff_view, move |me, view, event, ctx| {
            match event {
                CodeDiffViewEvent::TryAccept => {
                    view.update(ctx, |diff_view, ctx| {
                        diff_view.accept_and_save(ctx);
                    });
                }
                CodeDiffViewEvent::SavedAcceptedDiffs { .. } => {
                    ctx.notify();
                }
                CodeDiffViewEvent::CancelPassive => {
                    me.model
                        .lock()
                        .block_list_mut()
                        .remove_rich_content(wrapper_view_id);
                    me.rich_content_views
                        .retain(|rc| rc.view_id() != wrapper_view_id);
                    ctx.notify();
                }
                CodeDiffViewEvent::ContinuePassiveCodeDiffWithAgent { accepted } => {
                    let conversation_id = if let Some(conversation_id) = conversation_id {
                        conversation_id
                    } else {
                        // No existing conversation (ephemeral shell-command trigger): start a
                        // new one and open the agent view.
                        match me.try_enter_agent_view(
                            None,
                            AgentViewEntryOrigin::AcceptedPassiveCodeDiff,
                            None,
                            ctx,
                        ) {
                            Ok(conversation_id) => {
                                if let Some(block_id) = trigger_block_id.as_ref() {
                                    me.associate_and_promote_block_for_conversation(
                                        block_id.clone(),
                                        conversation_id,
                                        ctx,
                                    );
                                }
                                me.set_rich_content_agent_view_conversation_id(
                                    wrapper_view_id,
                                    conversation_id,
                                );
                                conversation_id
                            }
                            Err(e) => {
                                log::error!(
                                    "Failed to enter agent view for passive code diff: {e:?}"
                                );
                                return;
                            }
                        }
                    };

                    // Use the passive diff summary as the conversation title.
                    if let Some(title) = title_for_result.as_ref() {
                        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, _ctx| {
                            if let Some(conversation) = history.conversation_mut(&conversation_id) {
                                conversation.set_fallback_display_title(title.clone());
                            }
                        });
                    }

                    let summary = title_for_result.clone().unwrap_or_default();
                    let diffs = original_edits.clone();
                    if *accepted {
                        me.ai_controller.update(ctx, |controller, ctx| {
                            controller.send_passive_suggestion_result(
                                Some(conversation_id),
                                PassiveSuggestionResultType::CodeDiff {
                                    diffs,
                                    summary,
                                    accepted: true,
                                },
                                Some(trigger.clone()),
                                ctx,
                            );
                        });
                    } else {
                        // Queue the result so it's included with the next
                        // user-initiated request on this conversation.
                        me.ai_controller.update(ctx, |controller, _ctx| {
                            controller.queue_passive_suggestion_result(
                                conversation_id,
                                PassiveSuggestionResultType::CodeDiff {
                                    diffs,
                                    summary,
                                    accepted: false,
                                },
                                Some(trigger.clone()),
                            );
                        });
                    }
                }
                CodeDiffViewEvent::EditModeChanged { enabled } => {
                    if *enabled {
                        me.open_code_diff(view.clone(), ctx);
                    }
                    ctx.notify();
                }
                CodeDiffViewEvent::ToggleCodeReviewPane { entrypoint } => {
                    me.toggle_code_review_pane(
                        GitDeltaPreference::Always,
                        *entrypoint,
                        None,
                        true,
                        ctx,
                    );
                }
                CodeDiffViewEvent::DisplayModeChanged => {
                    // Re-render wrapper when the diff view expands/collapses.
                    ctx.notify();
                }
                CodeDiffViewEvent::Blur => {
                    me.focus_terminal(ctx);
                }
                _ => {}
            }
        });

        let suggestion_id = Uuid::new_v4().to_string();
        send_telemetry_from_ctx!(
            TelemetryEvent::SuggestedCodeDiffBannerShown {
                prompt_suggestion_id: suggestion_id,
                code_exchange_id: None,
                block_id: trigger_block_id_str,
                request_duration_ms,
                server_request_token,
            },
            ctx
        );

        self.insert_rich_content(
            None,
            wrapper_view,
            None,
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: true,
            },
            ctx,
        );
    }

    fn on_legacy_prompt_suggestion_generated(
        &mut self,
        prompt_suggestion: AgentModePromptSuggestion,
        block_id: BlockId,
        command: String,
        request_duration_ms: u64,
        ctx: &mut ViewContext<TerminalView>,
    ) {
        match prompt_suggestion {
            AgentModePromptSuggestion::Success(suggestion) => {
                if suggestion.prompt.is_empty() {
                    return;
                }

                let (query_string, block_command) = if should_collect_ai_ugc_telemetry(
                    ctx,
                    PrivacySettings::as_ref(ctx).is_telemetry_enabled,
                ) {
                    (Some(suggestion.prompt.to_string()), Some(command))
                } else {
                    (None, None)
                };

                let banner_id = self.inline_banners_state.next_banner_id();

                self.clear_prompt_suggestions(ctx);

                // Don't show banner if is coding query
                let is_coding_query =
                    suggestion.is_coding_query() && Self::passive_code_diffs_enabled(ctx);
                let static_prompt_suggestion_name =
                    suggestion.static_prompt_suggestion_name.clone();
                let suggestion_id = suggestion.id.clone();

                let trigger = {
                    let model = self.model.lock();
                    let Some(block_context) =
                        block_context_from_terminal_model(&model, &block_id, false)
                    else {
                        return;
                    };
                    PassiveSuggestionTrigger::ShellCommandCompleted(ShellCommandCompletedTrigger {
                        executed_shell_command: Box::new(block_context),
                        relevant_files: vec![],
                    })
                };

                let banner_state = PromptSuggestionBannerState {
                    banner_id,
                    prompt_suggestion: suggestion,
                    accept_button_mouse_state: Default::default(),
                    llm_warning_learn_more_hyperlink: Default::default(),
                    should_hide: is_coding_query,
                    trigger: Some(trigger),
                    conversation_id: None,
                    server_request_token: None,
                };

                self.inline_banners_state.prompt_suggestions_banner = Some(banner_state.clone());

                self.input.update(ctx, |input, ctx| {
                    input.set_prompt_suggestions_banner_state(Some(banner_state), ctx);
                    input.notify_and_notify_children(ctx);
                });

                if let Some(static_name) = static_prompt_suggestion_name {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::StaticPromptSuggestionsBannerShown {
                            id: suggestion_id,
                            query: query_string,
                            block_id: block_id.to_string(),
                            block_command,
                            static_prompt_suggestion_name: static_name,
                            request_duration_ms,
                            view: self.prompt_suggestion_view_type(ctx),
                        },
                        ctx
                    );
                } else {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::PromptSuggestionShown {
                            id: suggestion_id,
                            request_duration_ms,
                            block_id: Some(block_id.to_string()),
                            view: self.prompt_suggestion_view_type(ctx),
                            server_request_token: None,
                        },
                        ctx
                    );
                }

                ctx.notify();
            }
            AgentModePromptSuggestion::None | AgentModePromptSuggestion::Error => {}
        }
    }

    /// Generates command corrections, if applicable.
    fn maybe_generate_command_suggestions(
        &mut self,
        block_completed: &UserBlockCompleted,
        ctx: &mut ViewContext<TerminalView>,
    ) {
        let block_completed = block_completed.to_owned();

        if *InputSettings::as_ref(ctx).command_corrections.value() {
            let session_id = self.active_block_session_id();

            let session = session_id.and_then(|id| self.sessions.as_ref(ctx).get(id));
            let history_entries = session_id
                .and_then(|id| History::as_ref(ctx).commands(id))
                .into_iter()
                .flatten()
                .cloned()
                .collect();

            let _ = ctx.spawn(
                Self::fetch_command_corrections(block_completed.clone(), session, history_entries),
                Self::after_command_correction_generation,
            );
        }
    }

    fn can_suggest_alias_expansion(&mut self, ctx: &mut ViewContext<TerminalView>) -> bool {
        let has_user_seen_banner: bool = ctx
            .private_user_preferences()
            .read_value(ALIAS_EXPANSION_BANNER_SEEN_KEY)
            .unwrap_or_default()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(false);
        let alias_expansion_settings = AliasExpansionSettings::as_ref(ctx);
        let supported_on_current_platform = alias_expansion_settings
            .alias_expansion_enabled
            .is_supported_on_current_platform();
        let is_fish_shell = self
            .active_block_session_id()
            .and_then(|id| self.sessions.as_ref(ctx).get(id))
            .is_some_and(|s| s.shell().shell_type() == ShellType::Fish);

        supported_on_current_platform
            && !*alias_expansion_settings.alias_expansion_enabled
            && !has_user_seen_banner
            // We don't suggest alias expansions for fish since we already expand
            // abbreviations by default.
            && !is_fish_shell
    }

    fn maybe_suggest_alias_expansion(
        &mut self,
        block_completed: &UserBlockCompleted,
        ctx: &mut ViewContext<TerminalView>,
    ) {
        if let Some(session) = self
            .active_block_session_id()
            .and_then(|id| self.sessions.as_ref(ctx).get(id))
        {
            let command = block_completed.command.clone();
            ctx.spawn(
                async move { check_for_alias_async(&command, session).await },
                move |view, aliased_command, ctx| {
                    view.suggest_alias_expansion(aliased_command, ctx);
                },
            );
        }
    }

    fn suggest_alias_expansion(
        &mut self,
        aliased_command: Option<AliasedCommand>,
        ctx: &mut ViewContext<TerminalView>,
    ) {
        if let Some(aliased_command) = aliased_command {
            let _ = ctx
                .private_user_preferences()
                .write_value(ALIAS_EXPANSION_BANNER_SEEN_KEY, "true".to_owned());
            self.insert_alias_expansion_banner(aliased_command, ctx);
        }
    }

    fn maybe_send_block_completed_notification(
        &mut self,
        block: &UserBlockCompleted,
        block_duration: Duration,
        ctx: &mut ViewContext<TerminalView>,
    ) {
        let session_settings_handle = SessionSettings::as_ref(ctx);

        // If notifications are not enabled on this platform, we don't want to
        // send notifications or show any of the notification-related banners.
        if !session_settings_handle
            .notifications
            .is_supported_on_current_platform()
        {
            return;
        }

        // Don't send notifications for commands executed by an agent
        if block.was_part_of_agent_interaction {
            return;
        }

        let notification_settings = session_settings_handle.notifications.value().clone();
        let long_running_trigger = NotificationsTrigger::LongRunningCommand(
            !block.serialized_block.has_failed(),
            block_duration,
        );
        match notification_settings.mode {
            NotificationsMode::Unset => {
                if let NotificationsDiscoveryBanner::Triggered(trigger) =
                    self.inline_banners_state.notifications_discovery_banner
                {
                    // if the banner is not yet open, but there is some trigger,
                    // we were likely waiting on the block to finish so insert it now
                    self.insert_notifications_discovery_banner(trigger, ctx);
                } else if self.is_navigated_away_from_window(ctx)
                    && block_duration >= *DEFAULT_THRESHOLD_FOR_LONG_RUNNING_NOTIFICATION
                {
                    // otherwise, if the user is navigated away when the block completes
                    // and the block ran longer than the default for long-running notifications,
                    // insert a banner for the long running trigger
                    self.insert_notifications_discovery_banner(long_running_trigger, ctx);
                }
            }
            NotificationsMode::Enabled => {
                // If the notifications error is not open but was triggered,
                // we should insert a banner to surface the error
                if let NotificationsErrorBannerType::Triggered = self
                    .inline_banners_state
                    .notifications_error_banner
                    .banner_type
                {
                    self.insert_notifications_error_banner(ctx);
                } else if self.is_navigated_away_from_window(ctx)
                    && notification_settings.is_long_running_enabled
                    && block_duration >= notification_settings.long_running_threshold
                {
                    // Otherwise, since the block completed, check if we
                    // should send a notification for long-running command
                    let notification_content = long_running_trigger.create_notification_content(
                        block.command.clone(),
                        // Only include the last line when displaying a notification.
                        block
                            .output_truncated
                            .lines()
                            .last()
                            .map_or_else(String::new, ToOwned::to_owned),
                    );
                    ctx.emit(Event::SendNotification(notification_content));
                    send_telemetry_from_ctx!(
                        TelemetryEvent::NotificationSent {
                            trigger: long_running_trigger,
                            agent_variant: None,
                        },
                        ctx
                    );
                }
            }
            _ => {}
        }
    }

    /// Send a desktop notification that agent mode needs attention or has finished,
    /// otherwise insert a callout banner if notifications are unset.
    /// May become separate triggers if we show sub-tasks in the UI.
    /// Note that this does NOT handle agent mode toast notifications in-app.
    /// Those are handled in the workspace view on AgentManagementEvent::ConversationNeedsAttention.
    fn maybe_send_agent_mode_desktop_notification(
        &mut self,
        conversation_id: &AIConversationId,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.is_navigated_away_from_window(ctx) {
            return;
        }

        let Some(conversation) = BlocklistAIHistoryModel::as_ref(ctx).conversation(conversation_id)
        else {
            return;
        };
        if conversation.is_entirely_passive()
            || !conversation.status().should_trigger_notification()
        {
            return;
        }

        let Some(block_summary) = self.get_ai_notification_summary(conversation, ctx) else {
            return;
        };

        let trigger = if conversation.status().is_blocked() {
            NotificationsTrigger::NeedsAttention
        } else {
            NotificationsTrigger::AgentTaskCompleted(block_summary.success)
        };
        self.send_agent_desktop_notification_or_show_banner(
            trigger,
            block_summary.title,
            block_summary.description,
            Some(NotificationAgentVariant::Oz),
            ctx,
        );
    }

    /// Shared logic for sending a desktop notification (or showing a discovery banner)
    /// for any agent status change (both Warp's agent and any CLI agent).
    fn send_agent_desktop_notification_or_show_banner(
        &mut self,
        trigger: NotificationsTrigger,
        title: String,
        description: String,
        agent_variant: Option<NotificationAgentVariant>,
        ctx: &mut ViewContext<Self>,
    ) {
        let notification_settings = SessionSettings::as_ref(ctx).notifications.value().clone();

        match notification_settings.mode {
            NotificationsMode::Unset => {
                if let NotificationsDiscoveryBanner::Triggered(trigger) =
                    self.inline_banners_state.notifications_discovery_banner
                {
                    // if the banner is not yet open, but there is some trigger,
                    // we were likely waiting on the block to finish so insert it now
                    self.insert_notifications_discovery_banner(trigger, ctx);
                } else {
                    // otherwise, insert a discovery banner for the current trigger
                    self.insert_notifications_discovery_banner(trigger, ctx);
                }
            }
            NotificationsMode::Enabled => {
                let success = matches!(trigger, NotificationsTrigger::AgentTaskCompleted(true));
                if success {
                    if !notification_settings.is_agent_task_completed_enabled {
                        return;
                    }
                } else if !notification_settings.is_needs_attention_enabled {
                    return;
                }
                let notification_content = trigger.create_notification_content(title, description);
                ctx.emit(Event::SendNotification(notification_content));
                send_telemetry_from_ctx!(
                    TelemetryEvent::NotificationSent {
                        trigger,
                        agent_variant,
                    },
                    ctx
                );
            }
            _ => {}
        }
    }

    /// Executes a command that was submitted by the user and not yet sent to the shell.
    #[cfg(not(target_family = "wasm"))]
    pub(super) fn on_shell_determined(&self, ctx: &mut ViewContext<Self>) {
        if !self.model.lock().shared_session_status().is_viewer() {
            // Start a timer for the initial session bootstrapping, so that we can log and show a
            // banner to the user if the bootstrapping takes too long
            self.start_bootstrap_timer(BOOTSTRAP_FAILED_DURATION, ctx);
        }
    }
    #[cfg(not(target_family = "wasm"))]
    pub(super) fn on_pty_spawn_failed(
        &mut self,
        pty_spawn_error: anyhow::Error,
        ctx: &mut ViewContext<Self>,
    ) {
        self.pty_spawn_failed = true;
        self.insert_shell_process_terminated_banner(
            shell_terminated_banner::TerminationType::PtySpawnFailure { pty_spawn_error },
            ctx,
        );
        ctx.notify();
    }
    #[cfg(feature = "local_fs")]
    fn open_file_path(
        &mut self,
        path: PathBuf,
        line_and_column_num: Option<LineAndColumnArg>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();

        let settings = EditorSettings::as_ref(ctx);
        let target = resolve_file_target(&path, settings, None);

        ctx.emit(Event::OpenFileWithTarget {
            path,
            target,
            line_col: line_and_column_num,
        });
    }
    #[cfg(feature = "local_fs")]
    fn open_file_path_with_target(
        &mut self,
        path: PathBuf,
        target: FileTarget,
        line_and_column_num: Option<LineAndColumnArg>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
        ctx.emit(Event::OpenFileWithTarget {
            path,
            target,
            line_col: line_and_column_num,
        });
    }
    #[cfg(feature = "local_fs")]
    fn open_code_in_warp(
        &mut self,
        source: CodeSource,
        layout: EditorLayout,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(Event::OpenCodeInWarp { source, layout })
    }
    #[cfg(test)]
    pub fn clear_buffer_for_testing(&mut self, ctx: &mut ViewContext<Self>) {
        self.clear_buffer(ctx);
    }
    fn terminal_up(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.selected_blocks.is_empty() {
            let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
            match input_mode {
                InputMode::PinnedToBottom | InputMode::Waterfall => {
                    self.select_less_recent_block(false /* is_shift_down */, ctx);
                }
                InputMode::PinnedToTop => {
                    self.select_more_recent_block(
                        false, /* is_cmd_down */
                        false, /* is_shift_down */
                        ctx,
                    );
                }
            }
        } else if self.is_long_running() {
            self.on_ssh_warpification_key_event(None, ctx);
            let sequence =
                EscCodes::build_escape_sequence(self.model.lock().deref(), &[EscCodes::ARROW_UP]);
            self.write_user_bytes_to_pty(sequence, ctx);
        }
    }

    fn bookmark_up(&mut self, ctx: &mut ViewContext<Self>) {
        let next_index = self
            .selected_blocks
            .tail()
            .and_then(|selected_block_index| {
                let mut maximum_index_above_bookmark = None;
                for index in self.bookmarked_blocks.keys() {
                    if *index < selected_block_index {
                        if let Some(max_ind) = maximum_index_above_bookmark {
                            if *index > max_ind {
                                maximum_index_above_bookmark = Some(*index);
                            }
                        } else {
                            maximum_index_above_bookmark = Some(*index);
                        }
                    }
                }
                maximum_index_above_bookmark
            })
            .or_else(|| self.bookmarked_blocks.keys().max().copied());

        if let Some(index) = next_index {
            self.reset_selection_to_single_block(index, ctx);
            self.jump_to_previous_command(index, ctx);
            send_telemetry_from_ctx!(TelemetryEvent::JumpToBookmark, ctx);
            ctx.notify();
        }
    }

    fn bookmark_down(&mut self, ctx: &mut ViewContext<Self>) {
        let next_index = self
            .selected_blocks
            .tail()
            .and_then(|selected_block_index| {
                let mut minimum_index_below_bookmark = None;
                for index in self.bookmarked_blocks.keys() {
                    if *index > selected_block_index {
                        if let Some(min_ind) = minimum_index_below_bookmark {
                            if *index < min_ind {
                                minimum_index_below_bookmark = Some(*index);
                            }
                        } else {
                            minimum_index_below_bookmark = Some(*index);
                        }
                    }
                }
                minimum_index_below_bookmark
            })
            .or_else(|| self.bookmarked_blocks.keys().min().copied());

        if let Some(index) = next_index {
            self.reset_selection_to_single_block(index, ctx);
            self.jump_to_previous_command(index, ctx);
            send_telemetry_from_ctx!(TelemetryEvent::JumpToBookmark, ctx);
            ctx.notify();
        }
    }

    fn terminal_down(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.selected_blocks.is_empty() {
            let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
            match input_mode {
                InputMode::PinnedToBottom | InputMode::Waterfall => {
                    self.select_more_recent_block(
                        false, /* is_cmd_down */
                        false, /* is_shift_down */
                        ctx,
                    );
                }
                InputMode::PinnedToTop => {
                    self.select_less_recent_block(false /* is_cmd_down */, ctx);
                }
            }
        } else if self.is_long_running() {
            let sequence =
                EscCodes::build_escape_sequence(self.model.lock().deref(), &[EscCodes::ARROW_DOWN]);
            self.write_user_bytes_to_pty(sequence, ctx);
        }
    }

    fn page_up(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_long_running() {
            // Note: We explicitly use the CSI prefix, as the terminal we are impersonating
            // (`xterm-256color`) has the escape sequence for page up defined with that prefix
            let sequence = EscCodes::build_escape_sequence_with_c1(C1::CSI, EscCodes::PAGE_UP);
            self.write_user_bytes_to_pty(sequence, ctx);
        } else {
            self.update_scroll_position_locking(ScrollPositionUpdate::AfterPageUp, ctx);
        }
    }

    fn page_down(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_long_running() {
            // Note: We explicitly use the CSI prefix, as the terminal we are impersonating
            // (`xterm-256color`) has the escape sequence for page down defined with that prefix
            let sequence = EscCodes::build_escape_sequence_with_c1(C1::CSI, EscCodes::PAGE_DOWN);
            self.write_user_bytes_to_pty(sequence, ctx);
        } else {
            self.update_scroll_position_locking(ScrollPositionUpdate::AfterPageDown, ctx);
        }
    }

    fn move_home(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_long_running() {
            let sequence = EscCodes::build_escape_sequence(self.model.lock().deref(), b"H");
            self.write_user_bytes_to_pty(sequence, ctx);
        } else {
            self.update_scroll_position_locking(ScrollPositionUpdate::AfterHome, ctx);
        }
    }

    fn move_end(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_long_running() {
            let sequence = EscCodes::build_escape_sequence(self.model.lock().deref(), b"F");
            self.write_user_bytes_to_pty(sequence, ctx);
        } else {
            self.update_scroll_position_locking(ScrollPositionUpdate::AfterEnd, ctx);
        }
    }

    fn keyboard_select_text(
        &mut self,
        ctx: &mut ViewContext<Self>,
        direction: &SelectionDirection,
    ) {
        let semantic_selection = SemanticSelection::as_ref(ctx);
        let selection_result = self.model.lock().block_list_mut().move_selection_tail(
            direction,
            semantic_selection,
            self.is_inverted_blocklist(ctx),
        );

        if let Some(new_tail) = selection_result {
            // Because standardized endpoints fall in the vertical center of their row,
            // subtracting 0.5 positions us at the top of the row, where we'd like to scroll to.
            let row = new_tail.row - 0.5.into_lines();
            self.scroll_to_row_if_not_visible(row.into_lines(), ctx);
        }

        self.maybe_copy_selection_to_clipboard(ctx);

        // The text selection changed, so clear any previously attached context text.
        self.ai_context_model.update(ctx, |context_model, ctx| {
            context_model.set_pending_context_selected_text(None, false, ctx);
        });

        ctx.notify();
    }

    /// Takes a row in the blocklist coordinate space.
    fn scroll_to_row_if_not_visible(&mut self, row: Lines, ctx: &mut ViewContext<Self>) {
        self.update_scroll_position_locking(
            ScrollPositionUpdate::ScrollToBlocklistRowIfNotVisible { row },
            ctx,
        );
    }

    fn scroll_to_if_not_visible(&mut self, block_index: BlockIndex, ctx: &mut ViewContext<Self>) {
        let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();
        if !self.is_block_visible_locking(
            block_index,
            BlockVisibilityMode::TopOfBlockVisible,
            input_mode,
            ctx,
        ) {
            self.scroll_to(block_index, ctx);
        }
    }

    fn jump_to_previous_command(
        &mut self,
        topmost_block_index: BlockIndex,
        ctx: &mut ViewContext<Self>,
    ) {
        send_telemetry_from_ctx!(TelemetryEvent::JumpToPreviousCommand, ctx);
        self.scroll_to_if_not_visible(topmost_block_index, ctx);
    }

    fn jump_to_bookmark(&mut self, index: BlockIndex, ctx: &mut ViewContext<Self>) {
        self.reset_selection_to_single_block(index, ctx);
        self.jump_to_previous_command(index, ctx);

        send_telemetry_from_ctx!(TelemetryEvent::JumpToBookmark, ctx);

        ctx.notify();
    }

    /// Scrolls to the focused match.
    fn scroll_to_match(&mut self, ctx: &mut ViewContext<Self>) {
        // Scrolling to matches is not done for the alt screen.
        if self.model.lock().is_alt_screen_active() {
            return;
        }

        let Some(focused_match) = self.find_model.as_ref(ctx).focused_block_list_match() else {
            return;
        };
        let focused_match = &focused_match;

        let find_match_location = match focused_match {
            BlockListMatch::RichContent { index, .. } => {
                FindMatchScrollLocation::RichContent { index: *index }
            }
            BlockListMatch::CommandBlock(BlockGridMatch {
                block_index,
                range,
                grid_type,
                ..
            }) => {
                let focused_match_row = range.start().row;

                let block_section = match grid_type {
                    GridType::PromptAndCommand => {
                        BlockSection::PromptAndCommandGrid(focused_match_row.into_lines())
                    }
                    GridType::Output => BlockSection::OutputGrid(focused_match_row.into_lines()),
                    _ => {
                        // Find matches never occur in other grid types.
                        return;
                    }
                };
                FindMatchScrollLocation::Block {
                    block_index: *block_index,
                    section: block_section,
                }
            }
        };

        self.update_scroll_position_locking(
            ScrollPositionUpdate::ScrollToFindMatchIfNotVisible(find_match_location),
            ctx,
        );
    }

    /// Scrolls the view to the top of the block at `block_index`.
    fn scroll_to(&mut self, block_index: BlockIndex, ctx: &mut ViewContext<Self>) {
        self.update_scroll_position_locking(
            ScrollPositionUpdate::ScrollToTopOfBlock { block_index },
            ctx,
        );
    }

    /// Scrolls the view to the AI block associated with the given exchange ID.
    fn scroll_to_exchange(&mut self, exchange_id: AIAgentExchangeId, ctx: &mut ViewContext<Self>) {
        // Find the rich content view with the matching exchange_id.
        let Some(view_id) = self.rich_content_views.iter().find_map(|rc| {
            rc.ai_block_metadata()
                .filter(|meta| meta.exchange_id == exchange_id)
                .map(|_| rc.view_id())
        }) else {
            return;
        };

        // Get the TotalIndex from the model.
        let Some(index) = self
            .model
            .lock()
            .block_list()
            .removable_blocklist_item_position(&RemovableBlocklistItem::RichContent(view_id))
            .copied()
        else {
            return;
        };

        self.update_scroll_position_locking(
            ScrollPositionUpdate::ScrollToTopOfRichContent { index },
            ctx,
        );
    }

    #[cfg(any(test, feature = "integration_tests"))]
    pub fn selected_blocks_tail_index(&self) -> Option<BlockIndex> {
        self.selected_blocks.tail()
    }

    #[cfg(any(test, feature = "integration_tests"))]
    pub fn selected_blocks_pivot_index(&self) -> Option<BlockIndex> {
        self.selected_blocks
            .ranges()
            .last()
            .map(|range| range.pivot())
    }

    /// Inserts a dummy AI block with the given query and output strings.
    /// The directory is set to ~.
    #[cfg(any(test, feature = "integration_tests"))]
    pub fn insert_dummy_ai_block(
        &mut self,
        query: String,
        output: String,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<AIBlock> {
        use rand::distributions::{Alphanumeric, DistString};

        use crate::ai::agent::{
            AIAgentInput, AIAgentOutput, AIAgentOutputMessage, AIAgentText, AIAgentTextSection,
            MessageId, ServerOutputId,
        };
        use crate::ai::blocklist::FakeAIBlockModel;

        let inputs = vec![AIAgentInput::UserQuery {
            query,
            context: vec![AIAgentContext::Directory {
                pwd: Some("~".to_owned()),
                home_dir: None,
                are_file_symbols_indexed: false,
            }]
            .into(),
            static_query_type: None,
            referenced_attachments: Default::default(),
            user_query_mode: UserQueryMode::default(),
            running_command: None,
            intended_agent: None,
        }];

        let output = AIAgentOutput {
            messages: vec![AIAgentOutputMessage::text(
                MessageId::new("fake-id".to_owned()),
                AIAgentText {
                    sections: vec![AIAgentTextSection::PlainText {
                        text: output.into(),
                    }],
                },
            )],
            server_output_id: Some(ServerOutputId::new(format!(
                "test_output_id_{}",
                Alphanumeric.sample_string(&mut rand::thread_rng(), 24)
            ))),
            ..Default::default()
        };

        // Create a real conversation in the history model for this dummy block so it renders.
        let terminal_view_id = ctx.view_id();
        let mut new_conversation_id = None;
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, model_ctx| {
            let id =
                history.start_new_conversation(terminal_view_id, false, false, false, model_ctx);
            // Mark it active for good measure (not strictly required for rendering).
            history.set_active_conversation_id(id, terminal_view_id, model_ctx);
            new_conversation_id = Some(id);
        });
        let conversation_id = new_conversation_id.expect("conversation created for dummy AI block");

        let ai_block_model = Rc::new(FakeAIBlockModel::new(inputs, output));
        let ai_block = ctx.add_typed_action_view(|ctx| {
            AIBlock::new(
                ai_block_model,
                self.model.clone(),
                ClientIdentifiers {
                    client_exchange_id: Default::default(),
                    conversation_id,
                    response_stream_id: None,
                },
                self.ai_controller.clone(),
                self.get_relevant_files_controller.clone(),
                None,
                None,
                self.ai_action_model.clone(),
                self.ai_context_model.clone(),
                self.find_model.clone(),
                self.active_session.clone(),
                &self.cli_subagent_controller,
                &self.model_events_handle,
                self.agent_view_controller.clone(),
                self.ambient_agent_view_model.clone(),
                self.view_handle.clone(),
                ctx.view_id(),
                ctx,
            )
        });

        self.insert_rich_content(
            Some(RichContentType::AIBlock),
            ai_block.clone(),
            Some(RichContentMetadata::AIBlock(AIBlockMetadata {
                exchange_id: Default::default(),
                conversation_id,
                ai_block_handle: ai_block.clone(),
            })),
            RichContentInsertionPosition::Append {
                insert_below_long_running_block: false,
            },
            ctx,
        );
        ai_block
    }

    pub fn last_ai_block(&self) -> Option<ViewHandle<AIBlock>> {
        self.rich_content_views
            .iter()
            .rev()
            .find(|rc| !rc.is_usage_footer() && !rc.is_pending_user_query())
            .and_then(|rich_content| rich_content.ai_block_metadata())
            .map(|ai_metadata| ai_metadata.ai_block_handle.clone())
    }

    /// Returns the environment setup mode selector view handle for tab-level rendering.
    pub fn environment_setup_mode_selector_handle(
        &self,
    ) -> Option<&ViewHandle<EnvironmentSetupModeSelector>> {
        self.is_environment_setup_mode_selector_open
            .then_some(&self.environment_setup_mode_selector)
    }

    pub fn auth_secret_delete_confirmation_dialog_element(
        &self,
        ctx: &AppContext,
    ) -> Option<Box<dyn Element>> {
        self.input
            .as_ref(ctx)
            .auth_secret_delete_confirmation_dialog_element(ctx)
    }

    pub fn summarization_cancel_dialog_handle(
        &self,
        ctx: &AppContext,
    ) -> Option<ViewHandle<SummarizationCancelDialog>> {
        let agent_status_bar = self.input.as_ref(ctx).agent_status_bar().as_ref(ctx);
        agent_status_bar
            .should_show_summarization_cancel_dialog(ctx)
            .then(|| {
                agent_status_bar
                    .summarization_cancel_dialog_handle()
                    .clone()
            })
    }

    pub fn send_inline_review(
        &mut self,
        review_comments: AgentReviewCommentBatch,
        ctx: &mut ViewContext<Self>,
    ) -> anyhow::Result<()> {
        // Treat sending an inline review like executing a command/AI query for scrolling purposes.
        // This ensures that if the user was scrolled up in the blocklist, we scroll back to the
        // latest blocks so they can see the agent's activity.
        self.update_scroll_position_locking(
            ScrollPositionUpdate::AfterCommandExecutionStarted,
            ctx,
        );

        let context = self
            .ai_context_model
            .as_ref(ctx)
            .pending_context(ctx, true /* is_user_query */);

        let code_review_input = AIAgentInput::CodeReview {
            context: context.into(),
            review_comments,
        };

        if FeatureFlag::AgentView.is_enabled()
            && !self.agent_view_controller.as_ref(ctx).is_active()
        {
            self.enter_agent_view_for_new_conversation(
                None,
                AgentViewEntryOrigin::InlineCodeReview,
                ctx,
            );
        } else {
            // In general, user has expressed intent to "enter agent mode" by sending the inline review.
            // When NLD is on, this means unlocking any status locks similar to other agent mode queries.
            // When NLD is off, we override the input mode to AI.
            self.ai_input_model.update(ctx, |input_model, ctx| {
                input_model.set_input_config(
                    input_model
                        .input_config()
                        .with_input_type(InputType::AI)
                        .unlocked_if_autodetection_enabled(false, ctx),
                    true,
                    Some(InputTypeAutoDetectionSource::InlineCodeReviewSend),
                    ctx,
                );
            });
        }

        self.ai_controller.update(ctx, |controller, ctx| {
            // Send the code review request to the AI controller and return the result
            controller.send_custom_ai_input_query(code_review_input, ctx);
        });
        Ok(())
    }

    /// Returns the CLI agent currently active in this terminal, if any.
    pub fn active_cli_agent(&self, ctx: &AppContext) -> Option<super::CLIAgent> {
        if !FeatureFlag::HoaCodeReview.is_enabled() {
            return None;
        }

        CLIAgentSessionsModel::as_ref(ctx)
            .session(self.view_id)
            .map(|s| s.agent)
    }

    /// Returns `true` if CLI agent rich input is currently open.
    pub fn is_cli_agent_rich_input_open(&self, ctx: &AppContext) -> bool {
        CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.view_id)
    }

    /// Appends `text` to CLI agent rich input and focuses it.
    fn append_to_rich_input(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        self.input.update(ctx, |input, ctx| {
            input.append_to_buffer(text, ctx);
        });
        self.focus_input_box(ctx);
    }

    /// Sends `text` to the active CLI agent, routing to rich input when it is open
    /// or directly to the PTY when it is closed.
    ///
    /// Returns `Some(CliAgentRouting)` indicating how the text was sent, or
    /// `None` if no CLI agent is active.
    pub fn try_send_text_to_cli_agent_or_rich_input(
        &mut self,
        text: String,
        ctx: &mut ViewContext<Self>,
    ) -> Option<CliAgentRouting> {
        self.active_cli_agent(ctx)?;
        if self.is_cli_agent_rich_input_open(ctx) {
            self.append_to_rich_input(&text, ctx);
            Some(CliAgentRouting::RichInput)
        } else {
            self.write_to_pty(text.into_bytes(), ctx);
            self.focus_terminal(ctx);
            Some(CliAgentRouting::Pty)
        }
    }

    /// Sends code review comments to a running CLI agent, routing to the
    /// rich input when it is open or directly to the PTY when closed.
    pub fn send_review_to_cli_agent_or_rich_input(
        &mut self,
        review: &AgentReviewCommentBatch,
        ctx: &mut ViewContext<Self>,
    ) -> anyhow::Result<()> {
        let text = cli_agent::build_review_prompt(review);
        self.try_send_text_to_cli_agent_or_rich_input(text, ctx);
        Ok(())
    }

    /// Sends diff file context hunks to a running CLI agent, routing to the
    /// rich input when open or the PTY when closed.
    #[cfg(feature = "local_fs")]
    pub fn send_diff_context_to_cli_agent_or_rich_input(
        &mut self,
        file_diffs: &std::collections::HashMap<String, Vec<crate::ai::agent::DiffSetHunk>>,
        ctx: &mut ViewContext<Self>,
    ) -> Option<CliAgentRouting> {
        let text = cli_agent::build_diff_context_prompt(file_diffs);
        self.try_send_text_to_cli_agent_or_rich_input(text, ctx)
    }

    /// Sends a diff hunk location to a running CLI agent, routing to the
    /// rich input when open or the PTY when closed.
    pub fn send_diff_hunk_to_cli_agent_or_rich_input(
        &mut self,
        file_path: &str,
        start_line: usize,
        end_line: usize,
        lines_added: u32,
        lines_removed: u32,
        ctx: &mut ViewContext<Self>,
    ) -> Option<CliAgentRouting> {
        let text = cli_agent::build_diff_hunk_prompt(
            file_path,
            start_line,
            end_line,
            lines_added,
            lines_removed,
        );
        self.try_send_text_to_cli_agent_or_rich_input(text, ctx)
    }

    fn handle_theme_change(&mut self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let colors = color::List::from(&appearance.theme().clone().into());
        let mut model = self.model.lock();
        model.update_colors(colors);
        self.colors = colors;
        ctx.notify();
    }

    fn handle_reporting_settings_event(
        &mut self,
        _evt: &AltScreenReportingChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }

    fn handle_session_settings_event(
        &mut self,
        evt: &SessionSettingsChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match evt {
            SessionSettingsChangedEvent::HonorPS1 { .. } => {
                let session = self
                    .active_block_session_id()
                    .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id));

                if let Some(session) = session {
                    self.update_incompatible_configuration_banner(session.shell().plugins(), ctx)
                }

                // honor_ps1 affects whether the Warp prompt is active, which
                // determines if we need git status updates.
                self.update_git_status_subscription(ctx);
            }
            SessionSettingsChangedEvent::CLIAgentToolbarChipSelectionSetting { .. } => {
                // Force-close rich input when the Rich Input chip is removed so
                // it doesn't linger open with no toolbar button to manage it.
                if !is_rich_input_chip_in_cli_toolbar(ctx) {
                    self.close_cli_agent_rich_input(CLIAgentRichInputCloseReason::Other, ctx);
                }
                self.update_git_status_subscription(ctx);
            }
            SessionSettingsChangedEvent::AgentToolbarChipSelectionSetting { .. }
            | SessionSettingsChangedEvent::GithubPrChipDefaultValidation { .. } => {
                self.update_git_status_subscription(ctx);
            }
            _ => {}
        }
    }

    fn block_prompt(model: &TerminalModel, sessions: &Sessions, block_index: BlockIndex) -> String {
        let block = match model.block_list().block_at(block_index) {
            None => return String::new(),
            Some(block) => block,
        };

        let mut prompt = if block.honor_ps1() {
            block.prompt_contents_to_string(false)
        } else if block.prompt_snapshot().is_some() {
            // Note that we're checking not only for the flag being enabled but also ensuring the
            // prompt_snapshot is defined. This is because some historical blocks from the restored
            // session may not have yet their prompt_snapshot value, and we still want to show them
            // nicely.
            block
                .prompt_snapshot()
                .map(|prompt| prompt.to_string())
                .unwrap_or_default()
        } else {
            let session = block
                .session_id()
                .and_then(|session_id| sessions.get(session_id));
            let user_and_host_name_string = session.as_ref().and_then(|session| {
                prompt::user_and_host_name_string(
                    session.session_type().clone(),
                    session.hostname(),
                    session.user(),
                )
            });
            let home_dir = session
                .and_then(|session| session.home_dir().map(|directory| directory.to_owned()));

            format!(
                "{}{}{}{}{}",
                block
                    .conda_env()
                    .map_or_else(String::new, |b| format!("({b}) ")),
                block
                    .virtual_env_short_name()
                    .map_or_else(String::new, |b| format!("({b}) ")),
                user_and_host_name_string.unwrap_or_default(),
                prompt::display_path_string(block.pwd(), home_dir.as_deref()),
                block
                    .git_branch()
                    .map_or_else(String::new, |b| format!(" git:({b})")),
            )
        };

        // On Local and Dev channels, append an indicator when NLD was overridden.
        // Skip the honor_ps1 case since there's no good place to display the extra text.
        if !block.honor_ps1() && block.nld_overridden() && ChannelState::enable_debug_features() {
            prompt.push_str(" (nld overridden)");
        }

        prompt
    }

    /// Returns the duration as an std::time::Duration struct
    fn block_duration(&self, serialized_block: &SerializedBlock) -> Option<Duration> {
        (serialized_block.completed_ts? - serialized_block.start_ts?)
            .to_std()
            .ok()
    }

    fn block_duration_text(model: &TerminalModel, block_index: BlockIndex) -> Option<String> {
        model
            .block_list()
            .block_at(block_index)?
            .formatted_duration_string()
    }

    fn is_block_duration_live(model: &TerminalModel, block_index: BlockIndex) -> bool {
        model
            .block_list()
            .block_at(block_index)
            .is_some_and(|block| block.is_duration_live())
    }

    /// Returns `true` when the block is actively executing (has started but not
    /// yet completed). Used to kick off the repaint timer before the first
    /// whole-second tick so the live duration counter appears promptly.
    fn is_block_executing(model: &TerminalModel, block_index: BlockIndex) -> bool {
        model
            .block_list()
            .block_at(block_index)
            .is_some_and(|block| block.is_executing())
    }

    fn block_start_and_completed_ts(model: &TerminalModel, block_index: BlockIndex) -> String {
        let block = match model.block_list().block_at(block_index) {
            None => return String::new(),
            Some(block) => block,
        };

        let start = block.start_ts().map_or_else(String::new, |b| {
            format!("Started at: {}", b.format("%a %b %-d at %-I:%M:%S %p"))
        });
        let end = block.completed_ts().map_or_else(String::new, |b| {
            format!("\nCompleted at: {}", b.format("%a %b %-d at %-I:%M:%S %p"))
        });
        format!("{start}{end}")
    }

    #[cfg(any(test, feature = "integration_tests"))]
    pub fn is_find_bar_open(&self, app: &AppContext) -> bool {
        self.find_model.as_ref(app).is_find_bar_open()
    }

    pub fn is_find_bar_focused(&self, ctx: &AppContext) -> bool {
        self.find_bar.as_ref(ctx).is_editor_focused(ctx)
    }

    pub fn pwd(&self) -> Option<String> {
        self.active_block_metadata
            .as_ref()
            .and_then(BlockMetadata::current_working_directory)
            .map(|pwd| pwd.to_string())
    }

    pub fn pwd_if_local(&self, ctx: &AppContext) -> Option<String> {
        self.active_session_path_if_local(ctx)
            .map(|path| path.to_string_lossy().into_owned())
    }

    /// Returns the active session's CWD as a `LocalOrRemotePath`.
    ///
    /// For local sessions the CWD is canonicalized via `dunce::canonicalize`
    /// and wrapped as `Local`. For remote sessions the CWD is read from
    /// `active_block_metadata` and paired with the session's `host_id` to
    /// form a `Remote` path. Returns `None` when no CWD is available or
    /// (for remote sessions) the `host_id` has not been established yet.
    pub fn pwd_as_local_or_remote(&self, ctx: &AppContext) -> Option<LocalOrRemotePath> {
        let session_id = self.active_block_session_id()?;
        let session = self.sessions.as_ref(ctx).get(session_id)?;
        let cwd_str = self
            .active_block_metadata
            .as_ref()
            .and_then(BlockMetadata::current_working_directory)?;

        if self.session_is_local(session_id, ctx) {
            // Local session: canonicalize to resolve symlinks / normalize.
            let path = session
                .launch_data()
                .and_then(|data| data.maybe_convert_absolute_path(cwd_str))
                .unwrap_or_else(|| PathBuf::from(cwd_str));
            let canonical = dunce::canonicalize(&path).ok()?;
            Some(LocalOrRemotePath::Local(canonical))
        } else {
            // Remote session: pair CWD with the session's host_id.
            let host_id = match session.session_type() {
                SessionType::WarpifiedRemote { host_id } => host_id,
                SessionType::Local => return None,
            }?;
            let std_path = warp_util::standardized_path::StandardizedPath::try_new(cwd_str).ok()?;
            Some(LocalOrRemotePath::Remote(
                warp_util::remote_path::RemotePath::new(host_id, std_path),
            ))
        }
    }

    pub fn shell_launch_data_if_local(&self, ctx: &AppContext) -> Option<ShellLaunchData> {
        if !FeatureFlag::ShellSelector.is_enabled() {
            return None;
        }

        let session_id = self.active_block_session_id()?;
        let Some(session) = self.sessions.as_ref(ctx).get(session_id) else {
            log::warn!("Expected to have session for session ID {session_id:?}, but doesn't exist");
            return None;
        };
        if !session.is_local() {
            return None;
        }

        session.launch_data().cloned()
    }

    fn spawning_command_for_subshell_sessions(
        &self,
        app: &AppContext,
    ) -> HashMap<SessionId, SubshellSource> {
        self.sessions
            .as_ref(app)
            .spawning_command_for_subshell_sessions()
    }

    fn is_waterfall_gap_mode(&self, model: &TerminalModel, app: &AppContext) -> bool {
        let input_mode = *InputModeSettings::as_ref(app).input_mode.value();
        self.viewport_state(model.block_list(), input_mode, app)
            .is_waterfall_gap_mode()
    }

    pub fn get_terminal_view_render_context(
        &self,
        model: &TerminalModel,
        app: &AppContext,
    ) -> TerminalViewRenderContext {
        let (pane_state, active_session_state) = match self.focus_handle.as_ref() {
            Some(handle) => (
                handle.split_pane_state(app),
                if handle.is_active_session(app) {
                    ActiveSessionState::Active
                } else {
                    ActiveSessionState::Inactive
                },
            ),
            None => (SplitPaneState::NotInSplitPane, ActiveSessionState::Active),
        };

        TerminalViewRenderContext {
            size_info: *self.size_info(),
            scroll_position: self.scroll_position(),
            highlighted_url: self.highlighted_link.clone_inner(),
            link_tool_tip: self.open_grid_link_tool_tip.clone(),
            is_terminal_focused: self
                .view_handle
                .upgrade(app)
                .expect("terminal should upgrade")
                .is_focused(app),
            is_terminal_selecting: self.is_selecting(),
            is_context_menu_open: self.is_context_menu_open(),
            is_waterfall_gap_mode: self.is_waterfall_gap_mode(model, app),
            pane_state,
            active_session_state,
            selected_blocks: self.selected_blocks.clone(),
            input_box_element_key: self.input.as_ref(app).save_position_id(),
            terminal_view_id: self.view_id,
            spawning_command_for_subshell_sessions: self
                .spawning_command_for_subshell_sessions(app),
            obfuscate_secrets: get_secret_obfuscation_mode(app),
            hovered_secret: self.hovered_secret,
            horizontal_clipped_scroll_state: self.horizontal_clipped_scroll_state.clone(),
            ai_render_context: self.ai_render_context.clone(),
        }
    }

    #[cfg(feature = "integration_tests")]
    pub fn content_element_position_id(&self) -> &String {
        &self.content_element_position_id
    }
    #[cfg(feature = "integration_tests")]
    pub fn active_filter_editor_block_index(&self) -> Option<BlockIndex> {
        self.active_filter_editor_block_index
    }
    #[cfg(feature = "local_fs")]
    fn start_lsp_server_in_active_pwd(&self, ctx: &mut ViewContext<Self>) {
        use crate::ai::persisted_workspace::LspTask;

        let Some(cwd) = self
            .pwd_if_local(ctx)
            .map(PathBuf::from)
            .and_then(|p| p.canonicalize().ok())
        else {
            return;
        };

        PersistedWorkspace::handle(ctx).update(ctx, |workspace, ctx| {
            workspace.execute_lsp_task(LspTask::Spawn { file_path: cwd }, ctx);
        });
    }
    fn handle_action(&mut self, action: &TerminalAction, ctx: &mut ViewContext<Self>) {
        use TerminalAction::*;
        let input_mode = *InputModeSettings::as_ref(ctx).input_mode.value();

        match action {
            Scroll { delta } => self.scroll(*delta, ctx),
            AltScroll { delta } => self.alt_scroll(*delta, ctx),
            SharedSessionViewerAltScroll { new_scroll_top } => {
                self.alt_screen_scroll_top = *new_scroll_top;
                ctx.notify()
            }
            ScrollToTopOfBlock { topmost_block } => {
                self.jump_to_previous_command(*topmost_block, ctx)
            }
            ScrollToTopOfSelectedBlocks => self.scroll_to_top_of_topmost_selected_block(ctx),
            ScrollToBottomOfOverhangingBlock(overhanging_block) => {
                self.scroll_to_bottom_of_overhanging_block(overhanging_block, ctx)
            }
            ScrollToBottomOfSelectedBlocks => {
                self.scroll_to_bottom_of_bottommost_selected_block(ctx)
            }
            BlockTextSelect(select_action) => self.block_text_select(select_action, ctx),
            BlockSelect {
                action,
                should_redetermine_focus,
            } => self.block_select(action, *should_redetermine_focus, ctx),
            BlockHover(hover_action) => self.block_hover(hover_action, ctx),
            BlockSnackbarHover { is_hovered } => self.block_snackbar_hover(*is_hovered, ctx),
            BlockNearSnackbarHover { is_hovered } => {
                self.block_near_snackbar_hover(*is_hovered, ctx)
            }
            ClickOnGrid {
                position,
                modifiers,
            } => self.click_on_grid(position, modifiers, ctx),
            MaybeLinkHover {
                position,
                from_editor,
            } => self.maybe_link_hover(position, *from_editor, ctx),
            MaybeHoverSecret { secret_handle } => self.maybe_hover_secret(*secret_handle, ctx),
            MaybeDismissToolTip { from_keybinding } => {
                if !self.dismiss_tooltips(ctx) && *from_keybinding {
                    // If we are not dismissing the link tooltip, pass the esc escape sequence
                    // down to the terminal.
                    self.keydown_on_terminal("\u{1b}", ctx)
                }
            }
            MaybeClearAltSelect => {
                let mut model = self.model.lock();
                if model.alt_screen().selection().is_some() {
                    model.alt_screen_mut().clear_selection();
                    ctx.notify();
                }
            }
            AltSelect(select_action) => self.alt_select(select_action, ctx),
            AltScreenContextMenu { position } => self.alt_screen_context_menu(*position, ctx),
            AltMouseAction(mouse_state) => self.alt_mouse_action(mouse_state, ctx),
            BlockListContextMenu(menu_state) => self.block_list_context_menu(menu_state, ctx),
            OpenAIBlockAttachedBlocksMenu {
                exchange_id,
                conversation_id: ai_conversation_id,
                ai_block_view_id,
            } => self.open_ai_block_attached_context_menu(
                *ai_block_view_id,
                *exchange_id,
                *ai_conversation_id,
                ctx,
            ),
            OpenAIBlockOverflowMenu {
                exchange_id,
                conversation_id: ai_conversation_id,
                ai_block_view_id,
                is_restored,
            } => self.open_ai_block_overflow_context_menu(
                *ai_block_view_id,
                *exchange_id,
                *ai_conversation_id,
                *is_restored,
                ctx,
            ),
            RewindAIConversation {
                ai_block_view_id,
                exchange_id,
                conversation_id,
                entrypoint,
            } => {
                self.show_rewind_confirmation_dialog(
                    *ai_block_view_id,
                    *exchange_id,
                    *conversation_id,
                    *entrypoint,
                    ctx,
                );
            }
            ExecuteRewindAIConversation {
                ai_block_view_id,
                exchange_id,
                conversation_id,
            } => {
                self.rewind_ai_conversation(*ai_block_view_id, *exchange_id, *conversation_id, ctx)
            }
            ExecuteRewindFromInlineMenu {
                exchange_id,
                conversation_id,
            } => {
                // Find the ai_block_view_id for this exchange_id
                let ai_block_view_id = self.rich_content_views.iter().find_map(|rich_content| {
                    rich_content.ai_block_metadata().and_then(|metadata| {
                        if metadata.exchange_id == *exchange_id
                            && metadata.conversation_id == *conversation_id
                        {
                            Some(metadata.ai_block_handle.id())
                        } else {
                            None
                        }
                    })
                });

                if let Some(ai_block_view_id) = ai_block_view_id {
                    self.show_rewind_confirmation_dialog(
                        ai_block_view_id,
                        *exchange_id,
                        *conversation_id,
                        AgentModeRewindEntrypoint::SlashCommand,
                        ctx,
                    );
                } else {
                    log::warn!(
                        "Could not find AI block view for exchange_id {:?} in conversation {:?}",
                        exchange_id,
                        conversation_id
                    );
                }
            }
            CloseContextMenu => self.close_context_menu(ctx, true),
            Paste => self.paste(false, ctx),
            Copy => self.copy(ctx),
            CopyOutputs => self.copy_outputs(ctx),
            CopyCommands => self.copy_commands(ctx),
            CopyGitBranch => {
                let prompt_position = match self.selected_blocks.tail() {
                    Some(selected_block_index) => PromptPosition::Block(selected_block_index),
                    None => PromptPosition::Input,
                };
                self.copy_prompt(&prompt_position, &PromptPart::GitBranch, ctx)
            }
            OpenShareModal => self.open_share_block_modal(ctx),
            ReinputCommands => self.reinput_commands(false, ctx),
            ReinputCommandsWithSudo => self.reinput_commands(true, ctx),
            ClearBuffer => self.clear_buffer(ctx),
            Focus => self.redetermine_global_focus(ctx),
            FocusInputAndClearSelection => self.focus_input_and_clear_selections(ctx),
            ShowFindBar => self.show_find_bar(ctx),
            SelectPriorBlock => {
                let is_first_selection = self.selected_blocks.is_empty();
                match input_mode {
                    InputMode::PinnedToBottom | InputMode::Waterfall => {
                        self.select_less_recent_block(false /* is_shift_down */, ctx)
                    }
                    InputMode::PinnedToTop => {
                        self.select_more_recent_block(
                            true,  /* is_cmd_down */
                            false, /* is_shift_down */
                            ctx,
                        )
                    }
                }

                if is_first_selection && self.ai_input_model.as_ref(ctx).is_ai_input_enabled() {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::AgentModeAttachedBlockContext {
                            method: AgentModeAttachContextMethod::Keyboard
                        },
                        ctx
                    );
                }
            }
            SelectNextBlock => {
                match input_mode {
                    InputMode::PinnedToBottom | InputMode::Waterfall => self
                        .select_more_recent_block(
                            true,  /* is_cmd_down */
                            false, /* is_shift_down */
                            ctx,
                        ),
                    InputMode::PinnedToTop => {
                        self.select_less_recent_block(false /* is_shift_down */, ctx)
                    }
                }
            }
            Up => self.terminal_up(ctx),
            Down => self.terminal_down(ctx),
            PageUp => self.page_up(ctx),
            PageDown => self.page_down(ctx),
            Home => self.move_home(ctx),
            End => self.move_end(ctx),
            KeyboardSelectText(direction) => self.keyboard_select_text(ctx, direction),
            SelectBookmarkUp => match input_mode {
                InputMode::PinnedToBottom | InputMode::Waterfall => self.bookmark_up(ctx),
                InputMode::PinnedToTop => self.bookmark_down(ctx),
            },
            SelectBookmarkDown => match input_mode {
                InputMode::PinnedToBottom | InputMode::Waterfall => self.bookmark_down(ctx),
                InputMode::PinnedToTop => self.bookmark_up(ctx),
            },
            BookmarkSelectedBlock => self.bookmark_selected_block(ctx),
            UserInputSequence(bytes) => self.user_input_sequence(bytes, ctx),
            ControlSequence(bytes) => self.control_sequence_on_terminal(bytes, ctx),
            KeyDown(chars) => self.keydown_on_terminal(chars, ctx),
            TypedCharacters(chars) => self.typed_characters_on_terminal(chars, ctx),
            CtrlD => self.ctrl_d(ctx),
            CtrlC => self.handle_ctrl_c_input_event(0, ctx),
            ClearSelectionsWhenShellMode => self.clear_selections_when_shell_mode(ctx),
            ContextMenu(context_action) => self.context_menu_action(context_action, ctx),
            Close => ctx.emit(Event::CloseRequested),
            SplitRight(chosen_shell) => {
                ctx.emit(Event::Pane(PaneEvent::SplitRight(chosen_shell.to_owned())))
            }
            SplitLeft(chosen_shell) => {
                ctx.emit(Event::Pane(PaneEvent::SplitLeft(chosen_shell.to_owned())))
            }
            SplitDown(chosen_shell) => {
                ctx.emit(Event::Pane(PaneEvent::SplitDown(chosen_shell.to_owned())))
            }
            SplitUp(chosen_shell) => {
                ctx.emit(Event::Pane(PaneEvent::SplitUp(chosen_shell.to_owned())))
            }
            ToggleMaximizePane => ctx.emit(Event::Pane(PaneEvent::ToggleMaximized)),
            PromptContextMenu {
                position_offset_from_prompt,
            } => self.show_prompt_context_menu(*position_offset_from_prompt, ctx),
            OpenInputContextMenu { position } => self.show_input_context_menu(*position, ctx),
            InputContextMenuItem(action) => self.handle_input_context_menu_action(action, ctx),
            SelectAllBlocks => self.select_all_blocks(ctx),
            BookmarkBlock(index) => self.bookmark_block(index, ctx),
            ExpandBlockSelectionAbove => {
                match input_mode {
                    InputMode::PinnedToBottom | InputMode::Waterfall => {
                        self.select_less_recent_block(true /* is_shift_down */, ctx)
                    }
                    InputMode::PinnedToTop => {
                        self.select_more_recent_block(
                            false, /* is_cmd_down */
                            true,  /* is_shift_down */
                            ctx,
                        )
                    }
                }
            }
            ExpandBlockSelectionBelow => {
                match input_mode {
                    InputMode::PinnedToBottom | InputMode::Waterfall => self
                        .select_more_recent_block(
                            false, /* is_cmd_down */
                            true,  /* is_shift_down */
                            ctx,
                        ),
                    InputMode::PinnedToTop => {
                        self.select_less_recent_block(true /* is_shift_down */, ctx)
                    }
                }
            }
            NotificationsErrorBanner(action) => {
                self.notifications_error_banner_action(*action, ctx)
            }
            NotificationsDiscoveryBanner(action) => {
                self.notifications_discovery_banner_action(*action, ctx)
            }
            LegacySSHBanner(action) => self.ssh_banner_action(*action, ctx),
            JumpToBookmark(index) => self.jump_to_bookmark(*index, ctx),
            InsertCommandCorrection { correction } => {
                self.insert_command_correction(correction, ctx);
            }
            ToggleGridSecret {
                handle,
                show_secret,
            } => self.toggle_grid_secret(handle, *show_secret, ctx),
            ToggleRichContentSecret {
                rich_content_tooltip_info,
                show_secret,
            } => self.toggle_rich_content_secret(
                rich_content_tooltip_info.clone(),
                *show_secret,
                ctx,
            ),
            CopyGridSecret(secret_handle) => self.copy_grid_secret(secret_handle, ctx),
            CopyRichContentSecret(rich_content_tooltip_info) => {
                self.copy_rich_content_secret(rich_content_tooltip_info.clone(), ctx)
            }
            OpenGridLink(link) => {
                self.open_highlighted_link(link, ctx);
            }
            OpenRichContentLink(link) => {
                self.open_rich_content_link(link, ctx);
            }
            ShowInFileExplorer(path) => {
                send_telemetry_from_ctx!(TelemetryEvent::ShowInFileExplorer, ctx);

                ctx.open_file_path_in_explorer(path);
            }
            OpenFileInWarp(path) => {
                self.open_file_in_warp(path.clone(), ctx);
            }
            #[cfg(feature = "local_fs")]
            OpenCodeInWarp {
                path,
                layout,
                line_col,
            } => {
                self.open_code_in_warp(
                    CodeSource::Link {
                        path: path.clone(),
                        range_start: *line_col,
                        range_end: None,
                    },
                    *layout,
                    ctx,
                );
            }
            OpenWorkflowModal => self.open_workflow_modal(ctx),
            OpenWorkflowModalForAIWorkflow(workflow) => {
                self.open_workflow_modal_from_ai_generated_workflow(workflow.clone(), ctx)
            }
            OpenWorkflowModalForBlock(block_index) => {
                self.open_workflow_modal_from_block(*block_index, ctx)
            }
            OpenWorkflowModalWithCloudWorkflow(workflow_id) => {
                self.open_workflow_modal_with_existing(*workflow_id, ctx)
            }
            OpenBlockListContextMenu => self.open_block_list_context_menu_via_keybinding(ctx),
            AskAIAssistant { block_index } => {
                if FeatureFlag::AgentMode.is_enabled() {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::AgentModeClickedEntrypoint {
                            entrypoint: AgentModeEntrypoint::BlockToolbelt,
                        },
                        ctx
                    );
                }

                self.ask_ai(&AskAISource::Block(*block_index), ctx)
            }
            TriggerSubshellBootstrap => self.trigger_subshell_bootstrap(None, false, ctx),
            ShowSubshellBanner(command) => {
                // Abort handle is no longer needed since we've waited the 1s already.
                self.warpify_state.take_subshell_banner_abort_handle();

                let warpify_keybinding =
                    keybinding_name_to_keystroke("terminal:warpify_subshell", ctx);
                self.show_warpify_banner(
                    WarpificationMode::subshell(command.to_owned()),
                    "Subshell",
                    "subshell",
                    warpify_keybinding,
                    TelemetryEvent::ShowSubshellBanner,
                    ctx,
                );
            }
            ShowWarpifySshBanner(command, host) => {
                let warpify_keybinding =
                    keybinding_name_to_keystroke("terminal:warpify_ssh_session", ctx);
                self.show_warpify_banner(
                    WarpificationMode::ssh(command.to_string(), host.to_owned()),
                    "SSH Session",
                    "SSH session",
                    warpify_keybinding,
                    TelemetryEvent::SshTmuxWarpifyBannerDisplayed,
                    ctx,
                );
            }
            DismissWarpifyBanner(remember) => {
                self.dismiss_warpify_banner(remember, ctx);
                if remember.is_ssh() {
                    send_telemetry_from_ctx!(TelemetryEvent::SshTmuxWarpifyBlockDismissed, ctx);
                } else {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::DeclineSubshellBootstrap {
                            remember: remember.as_bool()
                        },
                        ctx
                    );
                }
            }
            InsertMostRecentCommandCorrection => self.insert_most_recent_command_correction(ctx),
            AliasExpansionBanner(action) => self.alias_expansion_banner_action(*action, ctx),
            OpenInWarpBanner(action) => self.handle_open_in_warp_banner_action(*action, ctx),
            OpenBlockFilterEditor(block_index) => {
                self.open_block_filter_editor(*block_index, OpenedFromClick::Yes, ctx)
            }
            VimModeBanner(action) => self.handle_vim_banner_action(*action, ctx),
            OnboardingFlow(version) => {
                // Don't show onboarding if it's already active or if this is a shared session or if user is anonymous
                if self
                    .model
                    .lock()
                    .shared_session_status()
                    .is_sharer_or_viewer()
                    || self.auth_state.is_anonymous_or_logged_out()
                {
                    return;
                };

                match version {
                    OnboardingVersion::Legacy => {
                        if self.block_onboarding_active {
                            return;
                        }

                        // We might want to consider marking the user as onboarded here,
                        // but it's probably fair to assume they have already been onboarded
                        // by the time they're manually triggering the onboarding flow.
                        self.add_agentic_suggestions_block(ctx);
                    }
                    OnboardingVersion::Agent(agent_version) => {
                        // The first Agent Modality callout expects terminal mode. If the
                        // default session mode is Agent (e.g. cloud-synced settings),
                        // the tab may already be in agent view — exit it first.
                        // This also removes any zero-state welcome blocks.
                        self.exit_agent_view(ctx);
                        self.start_agent_onboarding_tutorial(*agent_version, ctx);
                    }
                }
            }
            ImportSettings => {
                #[cfg(feature = "local_fs")]
                {
                    self.add_settings_import_block(ctx);
                    send_telemetry_from_ctx!(TelemetryEvent::SettingsImportInitiated, ctx);
                }
            }
            OpenShareSessionModal { source } => self.open_share_session_modal(*source, ctx),
            StopSharingCurrentSession { source } => self.stop_sharing_session(*source, ctx),
            ToggleBlockFilterOnSelectedOrLastBlock(source) => {
                self.toggle_block_filter_on_selected_or_last_block(*source, ctx);
            }
            CopySharedSessionLink { source } => self.copy_shared_session_link(*source, ctx),
            ToggleSnackbarInActivePane => self.toggle_snackbar_in_active_pane(ctx),
            MakeAllParticipantsReaders { reason } => {
                self.make_all_shared_session_participants_readers(*reason, ctx)
            }
            OpenSharedSessionViewerRoleMenu => self.open_shared_session_viewer_role_menu(ctx),
            RequestSharedSessionRole(role) => self.request_shared_session_role(*role, ctx),
            MiddleClickOnGrid { position } => self.middle_click_on_grid(position, ctx),
            MiddleClickOnInput => self.middle_click_on_input(ctx),
            OpenSharedSessionOnDesktop { source } => {
                self.open_shared_session_on_desktop(*source, ctx)
            }
            SelectAIAttachedBlock(block_index) => {
                self.scroll_to_and_maybe_select_block(*block_index, ctx)
            }
            DragAndDropFiles(paths) => {
                self.drag_and_drop_files(paths, ctx);
            }
            WarpifySSHSession => self.add_ssh_warpifying_block(ctx),
            NotifySshErrorBlock(action) => {
                if let Some(SshBlockState::Error {
                    handle: ssh_error_block_handle,
                }) = self.warpify_state.ssh_block_state()
                {
                    ssh_error_block_handle.update(ctx, |error_block, ctx| {
                        error_block.handle_action(action, ctx);
                    });
                }
            }
            SetInputModeAgent => {
                // Guard: when a CLI agent session is active, block mode
                // toggling and LRC subagent invocation. Context predicates
                // handle the Terminal-level case, but when the editor child
                // view is focused (rich input open via Ctrl-G), the parent's
                // CLI_AGENT_SESSION_ACTIVE_KEY flag isn't visible to the
                // keybinding matcher.
                if CLIAgentSessionsModel::as_ref(ctx)
                    .session(self.view_id)
                    .is_some()
                {
                    return;
                }
                if self
                    .model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_eligible_for_agent_handoff()
                {
                    self.cli_subagent_controller.update(ctx, |controller, ctx| {
                        controller.handoff_active_command_control_to_agent(ctx);
                    });
                } else if self
                    .model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_eligible_to_tag_in_agent()
                {
                    if FeatureFlag::AgentView.is_enabled() {
                        self.agent_view_controller.update(ctx, |controller, ctx| {
                            if !controller.is_inline() {
                                if let Err(e) = controller.try_enter_inline_agent_view(
                                    None,
                                    AgentViewEntryOrigin::LongRunningCommand,
                                    ctx,
                                ) {
                                    log::error!(
                                        "Failed to enter inline agent view for tag-in: {e}"
                                    );
                                }
                            }
                        });
                    }
                    self.tag_in_agent_for_user_long_running_command(ctx);
                } else {
                    self.input.update(ctx, |input, ctx| {
                        input.set_input_mode_agent(false, ctx);
                    });
                }
                ctx.notify();
            }
            SetInputModeTerminal => {
                if CLIAgentSessionsModel::as_ref(ctx)
                    .session(self.view_id)
                    .is_some()
                {
                    return;
                }
                if self
                    .model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_in_control()
                {
                    self.cli_subagent_controller.update(ctx, |controller, ctx| {
                        controller.switch_control_to_user(UserTakeOverReason::Manual, ctx);
                    });
                } else if self
                    .model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_tagged_in()
                {
                    self.tag_out_agent_for_user_long_running_command(ctx);

                    if FeatureFlag::AgentView.is_enabled()
                        && self.agent_view_controller.as_ref(ctx).is_inline()
                    {
                        self.agent_view_controller.update(ctx, |controller, ctx| {
                            controller.exit_agent_view(ctx);
                        });
                    }
                } else {
                    self.input.update(ctx, |input, ctx| {
                        input.set_input_mode_terminal(true, ctx);
                    });
                }
                ctx.notify();
            }
            #[cfg(feature = "voice_input")]
            ToggleCLIAgentVoiceInput(source) => {
                // For CLI agents, route through the footer's self-contained
                // voice flow (records + writes transcription to PTY). For
                // the regular editor, fall back to the editor-based flow.
                let has_cli_agent = self.use_agent_footer.as_ref(ctx).has_cli_agent(ctx);
                if has_cli_agent {
                    let footer = self.input.as_ref(ctx).agent_input_footer().clone();
                    footer.update(ctx, |footer, ctx| {
                        footer.toggle_cli_voice_input(source, ctx);
                    });
                } else {
                    self.input.update(ctx, |input, ctx| {
                        input.toggle_voice_input(source, ctx);
                    });
                }
            }
            HyperlinkClick(hyperlink) => {
                ctx.notify();
                ctx.open_url(&hyperlink.url);
            }
            AttemptLoginGatedFeature => {
                AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                    auth_manager.attempt_login_gated_feature(
                        "Upgrade AI Usage",
                        AuthViewVariant::RequireLoginCloseable,
                        ctx,
                    )
                });
            }
            StartFileDropTarget => {
                let Some(session) = self
                    .active_block_session_id()
                    .and_then(|session_id| self.sessions.as_ref(ctx).get(session_id))
                else {
                    return;
                };
                let sshed = self.model.lock().is_warpified_ssh() || session.is_legacy_ssh_session();
                if sshed && !self.is_file_drop_target {
                    self.is_file_drop_target = true;
                    ctx.notify();
                }
            }
            StopFileDropTarget => {
                if self.is_file_drop_target {
                    self.is_file_drop_target = false;
                    ctx.notify();
                }
            }
            RunNativeShellCompletions {
                buffer_text,
                results_tx,
            } => {
                ctx.emit(Event::RunNativeShellCompletions {
                    buffer_text: buffer_text.clone(),
                    results_tx: results_tx.clone(),
                });
            }
            OpenTeamSettingsPage => {
                ctx.emit(Event::OpenSettings(SettingsSection::Teams));
            }
            SetMarkedText {
                marked_text,
                selected_range,
            } => self.set_marked_text_on_terminal(marked_text, selected_range, ctx),
            ClearMarkedText => self.clear_marked_text_on_terminal(ctx),
            SelectAgenticSuggestion(index) => {
                if let Some(block) = self.onboarding_agentic_suggestions_block.as_ref() {
                    block.update(ctx, |block, ctx| {
                        block.handle_key_pressed(*index, ctx);
                    });
                }
            }
            HideTelemetryBannerPermanently => self.hide_telemetry_banner_permanently(ctx),
            ShowInitializationBlock => self.show_initialization_block(),
            GenerateCodebaseIndex => {
                self.generate_codebase_index(ctx);
            }
            LoadAgentModeConversation => {
                self.load_agent_mode_conversation(ctx);
            }
            ShowWarpifySettings => ctx.emit(Event::OpenSettings(SettingsSection::Warpify)),
            DeleteAttachment { index } => {
                self.ai_context_model.update(ctx, |context_model, ctx| {
                    context_model.remove_pending_attachment(*index, ctx);
                });
            }
            OpenAttachmentLightbox { index } => {
                let pending_images = self
                    .ai_context_model
                    .as_ref(ctx)
                    .pending_attachments()
                    .iter()
                    .enumerate()
                    .filter_map(|(attachment_index, attachment)| match attachment {
                        PendingAttachment::Image(image) => Some((attachment_index, image.clone())),
                        PendingAttachment::File(_) => None,
                    })
                    .collect::<Vec<_>>();
                let mut images = Vec::new();
                let mut initial_index = None;
                for (attachment_index, image) in pending_images {
                    let image_bytes =
                        match base64::engine::general_purpose::STANDARD.decode(&image.data) {
                            Ok(image_bytes) => image_bytes,
                            Err(error) => {
                                log::warn!(
                                "Failed to decode pending image attachment for lightbox: {error}"
                            );
                                continue;
                            }
                        };

                    let asset_id = format!("pending-attachment-lightbox-{attachment_index}");
                    AssetCache::handle(ctx).update(ctx, |asset_cache, ctx| {
                        asset_cache.insert_raw_asset_bytes::<ImageType>(
                            asset_id.clone(),
                            &image_bytes,
                            ctx,
                        );
                    });

                    if attachment_index == *index {
                        initial_index = Some(images.len());
                    }
                    images.push(ui_components::lightbox::LightboxImage {
                        source: ui_components::lightbox::LightboxImageSource::Resolved {
                            asset_source: warpui::assets::asset_cache::AssetSource::Raw {
                                id: asset_id,
                            },
                        },
                        description: Some(image.file_name.clone()),
                    });
                }

                let Some(initial_index) = initial_index else {
                    return;
                };

                ctx.dispatch_typed_action(&WorkspaceAction::OpenLightbox {
                    images,
                    initial_index,
                });
            }
            WriteCodebaseIndex => {
                self.write_codebase_index(ctx);
            }
            ToggleAutoexecuteMode => {
                // Cloud (ambient) agent conversations run with fast-forward conceptually
                // always on, so toggling it from the chip or keybinding is a no-op there.
                let is_locked = {
                    let terminal_model = self.model.lock();
                    is_in_cloud_context(
                        terminal_model.block_list().agent_view_state(),
                        &terminal_model,
                    )
                };
                if is_locked {
                    return;
                }

                // If there's a pending (blocked) requested code diff, accept it first.
                if let Some(ai_block) = self.last_ai_block() {
                    ai_block.update(ctx, |ai_block, ctx| {
                        ai_block.accept_pending_action(ctx);
                    });
                }

                self.ai_context_model.update(ctx, |context_model, ctx| {
                    context_model.toggle_pending_query_autoexecute(ctx);
                });
                ctx.notify();
            }
            ToggleQueueNextPrompt => {
                let Some(conversation_id) =
                    BlocklistAIHistoryModel::as_ref(ctx).active_conversation_id(self.view_id)
                else {
                    return;
                };
                QueuedQueryModel::handle(ctx).update(ctx, |model, ctx| {
                    model.toggle_queue_next_prompt(conversation_id, ctx);
                });
                ctx.notify();
            }
            CodebaseIndexSpeedbumpBanner(action) => {
                self.codebase_index_speedbump_banner_action(*action, ctx);
            }
            AgentModeSetupSpeedbumpBanner(action) => {
                self.agent_mode_setup_speedbump_banner_action(*action, ctx)
            }
            AnonymousUserAISignUpBanner(action) => {
                self.anonymous_user_ai_sign_up_banner_action(*action, ctx);
            }
            ResumeConversation => {
                // With Agent View, we want to resume the conversation the user is currently viewing,
                // not necessarily the most recently created one.
                let conversation_id = if FeatureFlag::AgentView.is_enabled() {
                    self.agent_view_controller
                        .as_ref(ctx)
                        .agent_view_state()
                        .active_conversation_id()
                } else {
                    BlocklistAIHistoryModel::as_ref(ctx).last_conversation_id(self.id())
                };
                if let Some(conversation_id) = conversation_id {
                    self.handle_resume_conversation(&conversation_id, ctx)
                }
            }
            ForkConversationFromLastKnownGoodState => {
                let active_conversation = if FeatureFlag::AgentView.is_enabled() {
                    self.agent_view_controller
                        .as_ref(ctx)
                        .agent_view_state()
                        .active_conversation_id()
                        .and_then(|id| BlocklistAIHistoryModel::as_ref(ctx).conversation(&id))
                } else {
                    BlocklistAIHistoryModel::as_ref(ctx).active_conversation(self.id())
                };
                if let Some(active_conversation) = active_conversation {
                    let conversation_id = active_conversation.id();
                    let exchange_id = {
                        let terminal_model = self.model.lock();
                        fork_from_last_known_good_state_exchange_id(
                            active_conversation,
                            &terminal_model,
                        )
                    };
                    if let Some(exchange_id) = exchange_id {
                        ctx.dispatch_typed_action(&WorkspaceAction::ForkAIConversation {
                            conversation_id,
                            fork_from_exchange: Some(ForkFromExchange {
                                exchange_id,
                                fork_from_exact_exchange: false,
                            }),
                            summarize_after_fork: false,
                            summarization_prompt: None,
                            initial_prompt: None,
                            destination: ForkedConversationDestination::SplitPane,
                        });
                    }
                }
            }
            ToggleAIDocumentPane => {
                if let Some(conversation) =
                    BlocklistAIHistoryModel::as_ref(ctx).active_conversation(self.id())
                {
                    let conversation_id = conversation.id();
                    let doc_model = AIDocumentModel::as_ref(ctx);
                    let is_plan_for_this_conversation_open = self
                        .agent_view_controller
                        .as_ref(ctx)
                        .pane_group_id()
                        .is_some_and(|pane_group_id| {
                            doc_model.is_document_visible_by_conversation_in_pane_group(
                                &conversation_id,
                                pane_group_id,
                            )
                        });
                    if is_plan_for_this_conversation_open {
                        ctx.emit(Event::HideAIDocumentPanes);
                    } else {
                        let docs = doc_model.get_all_documents_for_conversation(conversation_id);
                        match docs.len() {
                            0 => {} // No plans — nothing to do.
                            1 => {
                                let (document_id, doc) = &docs[0];
                                ctx.emit(Event::OpenAIDocumentPane {
                                    document_id: *document_id,
                                    document_version: doc.version,
                                    is_auto_open: false,
                                });
                            }
                            _ => {
                                // Multiple plans — open the plan picker menu.
                                self.input.update(ctx, |input, ctx| {
                                    input.open_plan_menu(conversation_id, ctx);
                                });
                            }
                        }
                    }
                }
            }
            ToggleTodoPopup => {
                self.is_todo_popup_visible = !self.is_todo_popup_visible;
                // Focus the todos popup for esc key handling
                if self.is_todo_popup_visible {
                    ctx.focus(&self.agent_todos_popup);
                }
                ctx.notify();
            }
            CloseTodoPopup => {
                self.is_todo_popup_visible = false;
                ctx.notify();
            }
            ToggleCodeReviewPane { entrypoint } => {
                ctx.emit(Event::ToggleCodeReviewPane(CodeReviewPanelArg {
                    repo_path: self.current_repo_path.clone(),
                    terminal_view: self.view_handle.clone(),
                    entrypoint: *entrypoint,
                    focus_new_pane: true,
                    cli_agent: None,
                }));
            }
            InitProject => self.init_project(false, ctx),
            SetupCloudEnvironment(repos) => {
                self.setup_cloud_environment(repos.clone(), ctx);
            }
            SetupCloudEnvironmentAndStart(repos) => {
                self.setup_cloud_environment_and_start(repos.clone(), ctx);
            }
            TriggerEnvironmentSetupSelection(repos) => {
                self.enter_environment_setup_selector(repos.clone(), ctx);
            }
            OpenEnvironmentManagementPane => {
                self.open_environment_management_pane(ctx);
            }
            SummarizeConversation => self.summarize_conversation(ctx),
            IndexProjectSpeedbump => {
                let codebase_context_enabled =
                    UserWorkspaces::as_ref(ctx).is_codebase_context_enabled(ctx);

                if FeatureFlag::FullSourceCodeEmbedding.is_enabled() && codebase_context_enabled {
                    #[cfg(feature = "local_fs")]
                    if let Some(current_dir) = self.pwd() {
                        let directory = PathBuf::from(&current_dir);

                        if let Ok(repo_path) = directory.canonicalize() {
                            // Start indexing the codebase
                            CodebaseIndexManager::handle(ctx).update(ctx, |manager, ctx| {
                                manager.index_directory(repo_path.clone(), ctx);
                            });

                            self.remove_codebase_index_speedbump_banner(ctx);
                            self.insert_codebase_index_speedbump_banner(
                                repo_path, true, /* show_is_indexing */
                                ctx,
                            );
                        }
                    }
                }
            }
            AddProjectAtCurrentDirectory => {
                // Get the current working directory and add it as a project
                if let Some(current_dir) = self.pwd() {
                    let path = PathBuf::from(&current_dir);

                    // Access the ProjectManagementModel and add the project
                    ProjectManagementModel::handle(ctx).update(ctx, |project_model, ctx| {
                        project_model.upsert_project(path, ctx);
                    });
                }
            }
            OpenProjectRulesPane => {
                if let Some(current_dir) = self.pwd() {
                    let mut warp_md_path = PathBuf::from(&current_dir);
                    warp_md_path.push(WARP_MD_PATH);
                    #[cfg(feature = "local_fs")]
                    ctx.emit(Event::OpenCodeInWarp {
                        source: CodeSource::ProjectRules { path: warp_md_path },
                        layout: *crate::util::file::external_editor::EditorSettings::as_ref(ctx)
                            .open_file_layout
                            .value(),
                    });
                }
            }
            OpenViewMCPPane => {
                ctx.emit(Event::OpenMCPSettingsPage {
                    page: Some(MCPServersSettingsPage::List),
                });
            }
            OpenAddMCPPane => {
                ctx.emit(Event::OpenMCPSettingsPage {
                    page: Some(MCPServersSettingsPage::Edit { item_id: None }),
                });
            }
            OpenBillingAndUsagePane => {
                ctx.emit(Event::OpenSettings(SettingsSection::BillingAndUsage));
            }
            OpenAddRulePane => {
                ctx.emit(Event::OpenAddRulePane);
            }
            OpenRulesPane => {
                ctx.emit(Event::OpenRulesPane);
            }
            OpenEditSkillPane { skill_reference } => {
                #[cfg(feature = "local_fs")]
                {
                    use ai::skills::SkillReference;

                    match skill_reference {
                        SkillReference::Path(path) => {
                            ctx.emit(Event::OpenCodeInWarp {
                                source: CodeSource::Skill {
                                    reference: skill_reference.clone(),
                                    path: path.clone(),
                                    origin: SkillOpenOrigin::OpenSkillCommand,
                                },
                                layout:
                                    *crate::util::file::external_editor::EditorSettings::as_ref(ctx)
                                        .open_file_layout
                                        .value(),
                            });
                        }
                        SkillReference::BundledSkillId(_) => {
                            let window_id = ctx.window_id();
                            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                                toast_stack.add_ephemeral_toast(
                                    DismissibleToast::error(
                                        "Bundled skills cannot be edited".to_string(),
                                    ),
                                    window_id,
                                    ctx,
                                );
                            });
                        }
                    }
                }

                #[cfg(not(feature = "local_fs"))]
                {
                    let _ = skill_reference;
                    let window_id = ctx.window_id();
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(
                                "Editing skills is not supported in this build".to_string(),
                            ),
                            window_id,
                            ctx,
                        );
                    });
                }
            }
            OpenAddPromptPane => ctx.emit(Event::OpenAddPromptPane {
                initial_content: None,
            }),
            PickRepoToOpen => {
                ctx.dispatch_typed_action(&WorkspaceAction::OpenRepository { path: None });
            }
            OpenFilesPalette { source } => ctx.emit(Event::OpenFilesPalette { source: *source }),
            DismissCodeToolbeltTooltip => {
                CodeSettings::handle(ctx).update(ctx, |settings, ctx| {
                    if let Err(e) = settings
                        .dismissed_code_toolbelt_new_feature_popup
                        .set_value(true, ctx)
                    {
                        log::warn!(
                            "Failed to mark code toolbelt new feature popup as dismissed: {e}"
                        );
                    }
                });
                ctx.notify();
            }
            StartLspServer => {
                #[cfg(feature = "local_fs")]
                self.start_lsp_server_in_active_pwd(ctx);
            }
            OpenConversationsPalette => {
                ctx.emit(Event::OpenConversationHistory);
            }
            ToggleLongRunningCommandControl => {
                let terminal_model = self.model.lock();
                let active_block = terminal_model.block_list().active_block();
                if active_block.is_agent_in_control() {
                    drop(terminal_model);
                    self.cli_subagent_controller.update(ctx, |controller, ctx| {
                        controller.switch_control_to_user(UserTakeOverReason::Manual, ctx);
                    });
                } else if active_block.is_eligible_for_agent_handoff() {
                    drop(terminal_model);
                    self.cli_subagent_controller.update(ctx, |controller, ctx| {
                        controller.handoff_active_command_control_to_agent(ctx);
                    });
                }
                ctx.notify();
            }
            ToggleHideCliResponses => {
                self.cli_subagent_controller.update(ctx, |controller, ctx| {
                    controller.toggle_hide_responses(ctx);
                });
                ctx.notify();
            }
            ExitAgentView => {
                // Match the back button's "for Orchestrator" affordance for
                // child agents: navigate to the parent before falling back
                // to the in-place exit flow.
                if self.try_navigate_to_parent_conversation(ctx) {
                    ctx.notify();
                } else if self
                    .agent_view_controller
                    .as_ref(ctx)
                    .can_exit_agent_view()
                    .is_ok()
                {
                    self.exit_agent_view(ctx);
                    ctx.notify();
                }
            }
            EnterCloudAgentView => {
                let mut draft_text = self.input.as_ref(ctx).buffer_text(ctx);
                draft_text.truncate(draft_text.trim_end().len());
                let initial_prompt = (!draft_text.trim().is_empty()).then_some(draft_text);
                self.enter_cloud_agent_view(initial_prompt, ctx);
            }
            StartNewAgentConversation => {
                self.input.update(ctx, |input, ctx| {
                    input.handle_action(&InputAction::StartNewAgentConversation, ctx);
                });
            }
            OpenInlineHistoryMenu => {
                self.input.update(ctx, |input, ctx| {
                    input.handle_action(&InputAction::OpenInlineHistoryMenu, ctx);
                });
            }
            OpenModelSelector => {
                self.input.update(ctx, |input, ctx| {
                    input.handle_action(&InputAction::OpenModelSelector, ctx);
                });
            }
            ResolvePromptSuggestion(resolution) => {
                self.resolve_passive_suggestion(*resolution, ctx);
            }
            AwsBedrockLoginBanner(action) => {
                self.handle_aws_bedrock_login_banner_action(*action, ctx);
            }
            AwsCliNotInstalledBanner(action) => {
                self.handle_aws_cli_not_installed_banner_action(*action, ctx);
            }
            ToggleConversationDetailsPanel => {
                let will_open = !self.is_conversation_details_panel_open;
                self.is_conversation_details_panel_open = will_open;
                if will_open {
                    self.fetch_and_update_conversation_details_panel(ctx);
                }
                ctx.notify();
            }
            CancelAmbientAgentTask => {
                if let Some(ambient_agent_view_model) = self.ambient_agent_view_model.as_ref() {
                    ambient_agent_view_model.update(ctx, |model, ctx| {
                        model.cancel_task(ctx);
                    });
                }
                ctx.notify();
            }
            ToggleUsageFooter => {
                self.toggle_usage_footer(ctx);
            }
            RevealChildAgent { conversation_id } => {
                ctx.emit(Event::RevealChildAgent {
                    conversation_id: *conversation_id,
                });
            }
            SwitchAgentViewToConversation { conversation_id } => {
                // Pill-bar nav: every child has a hidden pane, so swap to it.
                ctx.emit(Event::SwapPaneToConversation {
                    conversation_id: *conversation_id,
                });
            }
            OpenChildAgentInNewPane { conversation_id } => {
                // Reveal the existing child pane as a sibling; preserves
                // in-flight state. Don't touch `self`'s active conversation
                // — `self` is the child view, swapped into the orchestrator's slot.
                ctx.emit(Event::OpenChildAgentInNewPane {
                    conversation_id: *conversation_id,
                });
            }
            OpenChildAgentInNewTab { conversation_id } => {
                // Workspace re-parents the existing child pane into a new tab.
                // Don't touch `self`'s active conversation (same reason as above).
                ctx.emit(Event::OpenChildAgentInNewTab {
                    conversation_id: *conversation_id,
                });
            }
            StopAgentConversation { conversation_id } => {
                ctx.emit(Event::StopAgentConversation {
                    conversation_id: *conversation_id,
                });
            }
            KillAgentConversation { conversation_id } => {
                ctx.emit(Event::KillAgentConversation {
                    conversation_id: *conversation_id,
                });
            }
            ToggleSessionRecording => {
                self.pty_recorder.update(ctx, |recorder, ctx| {
                    recorder.toggle_recording(ctx);
                });
            }
            ToggleCLIAgentRichInput => {
                if self.has_active_cli_agent_input_session(ctx) {
                    self.close_cli_agent_rich_input_and_disable_auto_toggle(ctx);
                } else {
                    self.open_cli_agent_rich_input(CLIAgentInputEntrypoint::CtrlG, ctx);
                }
            }
        }
    }
}

impl View for TerminalView {
    fn ui_name() -> &'static str {
        "Terminal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        // Grab this here, before we take the terminal model lock.
        let menu_positioning = self.input.as_ref(app).menu_positioning(app);

        let appearance = Appearance::as_ref(app);
        let semantic_selection = SemanticSelection::as_ref(app);
        let model = self.model.lock();
        let input_mode = if FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(app).is_fullscreen()
        {
            // When in agent view, layout is always pin to bottom.
            InputMode::PinnedToBottom
        } else {
            *InputModeSettings::as_ref(app).input_mode.value()
        };
        let viewport = self.viewport_state(model.block_list(), input_mode, app);
        let is_alt_screen_active = { model.is_alt_screen_active() };
        // Compute callout positioning early while we have the model lock.
        // For the final Agent Modality callout, always position relative to the input box,
        // even when the zero state is visible.
        let should_position_callout_above_zero_state = self
            .onboarding_callout_view
            .as_ref()
            .is_some_and(|v| v.as_ref(app).should_position_above_zero_state(app));
        let is_long_running_command = {
            model
                .block_list()
                .active_block()
                .is_active_and_long_running()
        };

        let mut column = match input_mode {
            InputMode::PinnedToTop => Flex::column().with_reverse_orientation(),
            InputMode::PinnedToBottom | InputMode::Waterfall => Flex::column(),
        };

        let mut did_wrap_terminal_size = false;

        fn wrap_in_terminal_size_element(
            resize_tx: &Sender<Vector2F>,
            element: Box<dyn Element>,
        ) -> Box<dyn Element> {
            TerminalSizeElement::new(resize_tx.clone(), element).finish()
        }

        let mut stack = match (
            input_mode,
            model.block_list().active_gap(),
            is_alt_screen_active,
        ) {
            (InputMode::Waterfall, Some(active_gap), false) => {
                self.render_waterfall_gap_element(&model, &viewport, active_gap, appearance, app)
            }
            (input_mode, _, _) => {
                if self.input.as_ref(app).is_cloud_mode_input_v2_composing(app) {
                    column.add_child(Expanded::new(1., self.render_input()).finish());

                    Stack::new()
                        .with_constrain_absolute_children()
                        .with_child(column.finish())
                } else {
                    let output_area = if (model.shared_session_status().is_view_pending()
                        && !self.is_ambient_agent_session(app))
                        || model.is_loading_conversation_transcript()
                    {
                        self.render_viewer_loading(app)
                    } else if is_alt_screen_active {
                        did_wrap_terminal_size = true;
                        wrap_in_terminal_size_element(
                            &self.resize_tx,
                            self.render_alt_screen_element(
                                app,
                                &model,
                                model.alt_screen().selection_range(semantic_selection),
                            ),
                        )
                    } else {
                        self.render_block_list_element(&model, input_mode, true, app)
                    };

                    column.add_child(Shrinkable::new(1., output_area).finish());

                    if model.is_alt_screen_active()
                        && self.should_render_use_agent_footer(&model, app)
                    {
                        column.add_child(ChildView::new(&self.use_agent_footer).finish());
                    }

                    if self.is_input_box_visible(&model, app) {
                        column.add_child(self.render_input());
                    } else if self.should_render_legacy_ambient_agent_loading_footer(&model, app) {
                        column.add_child(ambient_agent::render_loading_footer(appearance));
                    } else if self.show_remote_server_loading_footer(&model, app) {
                        column.add_child(
                            self.render_remote_server_loading_footer(&model, appearance, app),
                        );
                    }

                    let stack = Stack::new()
                        .with_constrain_absolute_children()
                        .with_child(Clipped::new(column.finish()).finish());
                    if matches!(input_mode, InputMode::Waterfall) && !is_alt_screen_active {
                        self.render_waterfall_mode_background(&model, stack, app)
                    } else {
                        stack
                    }
                }
            }
        };

        if self.is_any_tooltip_open() {
            self.render_grid_tooltip(&mut stack, &model, appearance, app);
        }

        // Show progress steps while waiting for an ambient agent to start. CloudModeSetupV2 uses
        // the agent status bar for setup/follow-up progress.
        if self.ambient_agent_view_model.as_ref().is_some_and(|model| {
            let model = model.as_ref(app);
            model.agent_progress().is_some() && !FeatureFlag::CloudModeSetupV2.is_enabled()
        }) {
            stack.add_child(self.render_ambient_agent_progress(appearance, app));
        }

        // For shared session viewers, we want to show a "Request edit access"
        // button near the input if the input (or the button) are being hovered.
        // This is disabled when the viewer is offline.
        if let Some(Viewer {
            input_request_edit_access_button_handle,
            pending_role_request,
            is_reconnecting,
            ..
        }) = self.shared_session_viewer()
        {
            if model.shared_session_status().is_reader()
                && !*is_reconnecting
                && !pending_role_request
                && self.context_menu_state.is_none()
                && self.is_input_box_visible(&model, app)
                && (self
                    .input_hoverable_handle
                    .lock()
                    .is_ok_and(|handle| handle.is_hovered())
                    || input_request_edit_access_button_handle
                        .lock()
                        .is_ok_and(|handle| handle.is_hovered()))
            {
                // Position the button above / below the input depending
                // on the input model.
                let input_anchor = match input_mode {
                    InputMode::PinnedToBottom => PositionedElementAnchor::TopMiddle,
                    InputMode::PinnedToTop => PositionedElementAnchor::BottomMiddle,
                    InputMode::Waterfall => {
                        if model.block_list().active_gap().is_some() {
                            PositionedElementAnchor::BottomMiddle
                        } else {
                            PositionedElementAnchor::TopMiddle
                        }
                    }
                };
                stack.add_positioned_overlay_child(
                    self.render_input_request_edit_access_button(
                        input_request_edit_access_button_handle.clone(),
                        appearance,
                    ),
                    OffsetPositioning::offset_from_save_position_element(
                        self.input.as_ref(app).status_free_input_save_position_id(),
                        Vector2F::zero(),
                        PositionedElementOffsetBounds::WindowByPosition,
                        input_anchor,
                        ChildAnchor::Center,
                    ),
                );
            }
        }

        self.maybe_render_onboarding_callout(
            menu_positioning,
            should_position_callout_above_zero_state,
            &mut stack,
            app,
        );

        match &self.context_menu_state.map(|c| c.menu_type) {
            Some(ContextMenuType::BlockList { menu_source }) => match menu_source {
                BlockListMenuSource::BlockOverflowButton { block_index }
                | BlockListMenuSource::BlockKeybinding { block_index } => stack
                    .add_positioned_overlay_child(
                        ChildView::new(&self.context_menu).finish(),
                        OffsetPositioning::offset_from_save_position_element(
                            format!("context_menu_button_{block_index}").as_str(),
                            vec2f(OVERFLOW_BUTTON_OFFSET_X, 0.),
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::TopLeft,
                            ChildAnchor::TopRight,
                        ),
                    ),

                BlockListMenuSource::RegularBlockRightClick {
                    position_in_terminal_view,
                    ..
                }
                | BlockListMenuSource::RegularTextRightClick {
                    position_in_terminal_view,
                }
                | BlockListMenuSource::RichContentBlockRightClick {
                    position_in_terminal_view,
                    ..
                }
                | BlockListMenuSource::OutsideBlockRightClick {
                    position_in_terminal_view,
                } => stack.add_positioned_overlay_child(
                    ChildView::new(&self.context_menu).finish(),
                    OffsetPositioning::offset_from_parent(
                        *position_in_terminal_view,
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                ),

                BlockListMenuSource::RichContentTextRightClick {
                    rich_content_view_id,
                    position_in_rich_content,
                } => stack.add_positioned_overlay_child(
                    ChildView::new(&self.context_menu).finish(),
                    OffsetPositioning::offset_from_save_position_element(
                        get_rich_content_position_id(rich_content_view_id),
                        *position_in_rich_content,
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                ),
            },
            Some(ContextMenuType::Prompt { position }) => stack.add_positioned_overlay_child(
                ChildView::new(&self.context_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    format!("prompt_area_{}", self.input.id()),
                    *position,
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::TopLeft,
                    ChildAnchor::BottomLeft,
                ),
            ),
            Some(ContextMenuType::AltScreen { position }) => stack.add_positioned_overlay_child(
                ChildView::new(&self.context_menu).finish(),
                OffsetPositioning::offset_from_parent(
                    *position,
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopLeft,
                    ChildAnchor::TopLeft,
                ),
            ),
            Some(ContextMenuType::Input { position }) => stack.add_positioned_overlay_child(
                ChildView::new(&self.context_menu).finish(),
                match input_mode {
                    InputMode::PinnedToBottom => {
                        OffsetPositioning::offset_from_save_position_element(
                            self.input.as_ref(app).editor_save_position_id(),
                            *position,
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::TopLeft,
                            ChildAnchor::BottomLeft,
                        )
                    }
                    InputMode::PinnedToTop | InputMode::Waterfall => {
                        OffsetPositioning::offset_from_save_position_element(
                            self.input.as_ref(app).editor_save_position_id(),
                            *position,
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::TopLeft,
                            ChildAnchor::TopLeft,
                        )
                    }
                },
            ),
            Some(ContextMenuType::AIBlockAttachedContext { ai_block_view_id }) => stack
                .add_positioned_overlay_child(
                    ChildView::new(&self.context_menu).finish(),
                    OffsetPositioning::offset_from_save_position_element(
                        get_attached_blocks_chip_element_position_id(*ai_block_view_id),
                        vec2f(10., -10.),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    ),
                ),
            Some(ContextMenuType::AIBlockOverflowMenu { ai_block_view_id }) => stack
                .add_positioned_overlay_child(
                    ChildView::new(&self.context_menu).finish(),
                    OffsetPositioning::offset_from_save_position_element(
                        get_ai_block_overflow_menu_element_position_id(*ai_block_view_id),
                        vec2f(OVERFLOW_BUTTON_OFFSET_X, 0.),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::TopLeft,
                        ChildAnchor::TopRight,
                    ),
                ),
            Some(ContextMenuType::AgentViewEntryConversation {
                agent_view_entry_block_id,
                position,
            }) => stack.add_positioned_overlay_child(
                ChildView::new(&self.context_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    get_agent_view_entry_block_position_id(*agent_view_entry_block_id),
                    *position,
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::TopLeft,
                    ChildAnchor::TopLeft,
                ),
            ),
            None => {}
        }

        if self.find_model.as_ref(app).is_find_bar_open() {
            stack.add_child(ChildView::new(&self.find_bar).finish());
        }

        if let Some(active_filter_editor_block_index) = self.active_filter_editor_block_index {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.block_filter_editor).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    filter_button_position_id(active_filter_editor_block_index),
                    vec2f(34., 12.),
                    PositionedElementOffsetBounds::ParentByPosition,
                    PositionedElementAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
        }

        if let Some(reconnecting_banner) = self
            .shared_session
            .as_ref()
            .and_then(|s| s.reconnecting_banner())
        {
            stack.add_child(ChildView::new(reconnecting_banner).finish());
        } else if !model.shared_session_status().is_viewer() {
            // We don't care about these banners for shared session viewers.

            // Only show one of these banners at a time, to avoid them visually
            // stacking on top of each other.
            if self.is_slow_bootstrap_banner_open
                && ContextFlag::ShowSlowShellStartupBanner.is_enabled()
            {
                stack.add_child(ChildView::new(&self.slow_bootstrap_banner).finish());
            } else if self.control_master_error_banner_state.is_open {
                stack.add_child(ChildView::new(&self.control_master_error_banner).finish());
            } else if self.is_incompatible_configuration_banner_open {
                stack.add_child(ChildView::new(&self.incompatible_configuration_banner).finish());
            } else if self.is_emacs_bindings_banner_open {
                stack.add_child(ChildView::new(&self.emacs_bindings_banner).finish());
            }
        }

        let block_list_settings = BlockListSettings::handle(app);
        let is_jump_to_bottom_enabled = *block_list_settings
            .as_ref(app)
            .show_jump_to_bottom_of_block_button
            .value();
        if is_jump_to_bottom_enabled {
            if let Some(overhanging_block) = viewport.overhanging_bottom_block(app) {
                let button_hovered = self.is_jump_to_bottom_of_block_element_hovered();
                let block_hovered = self
                    .hovered_block_index
                    .is_some_and(|hovered_index| hovered_index == overhanging_block.block_index());
                if (button_hovered || block_hovered)
                    && overhanging_block.visible_block_height_px()
                        > *JUMP_TO_BOTTOM_OVERHANG_THRESHOLD_PX
                {
                    let positioning = match (input_mode, self.is_input_box_visible(&model, app)) {
                        (InputMode::PinnedToBottom | InputMode::Waterfall, true) => {
                            // In waterfall or pinned to bottom mode, the button is positioned relative to the top right
                            // of the input area
                            OffsetPositioning::offset_from_save_position_element(
                                self.input.as_ref(app).save_position_id(),
                                vec2f(-10. - SCROLLBAR_WIDTH.as_f32(), -10.),
                                PositionedElementOffsetBounds::WindowByPosition,
                                PositionedElementAnchor::TopRight,
                                ChildAnchor::BottomRight,
                            )
                        }
                        // In pinned to top mode or the input is not visible, the button is positioned relative to the bottom right
                        // of the parent element
                        (_, _) => OffsetPositioning::offset_from_parent(
                            vec2f(-10. - SCROLLBAR_WIDTH.as_f32(), -10.),
                            ParentOffsetBounds::ParentByPosition,
                            ParentAnchor::BottomRight,
                            ChildAnchor::BottomRight,
                        ),
                    };
                    stack.add_positioned_child(
                        self.render_jump_to_bottom_of_block_element(
                            overhanging_block,
                            is_long_running_command,
                            appearance,
                            app,
                        ),
                        positioning,
                    );
                }
            }
        }

        // Add a border above the input view when there's an overhanging block (or below in input at the top
        // mode).
        if ((viewport.overhanging_bottom_block(app).is_some()
            && FeatureFlag::MinimalistUI.is_enabled())
            || *BlockListSettings::as_ref(app).show_block_dividers.value())
            && self.is_input_box_visible(&model, app)
            && !self
                .input
                .as_ref(app)
                .should_show_universal_developer_input(app)
            && !(FeatureFlag::AgentView.is_enabled()
                && self.agent_view_controller.as_ref(app).is_fullscreen())
        {
            let positioning = match input_mode {
                InputMode::PinnedToBottom | InputMode::Waterfall => {
                    OffsetPositioning::offset_from_save_position_element(
                        self.input.as_ref(app).save_position_id(),
                        vec2f(0., 0.),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    )
                }
                _ => OffsetPositioning::offset_from_save_position_element(
                    self.input.as_ref(app).save_position_id(),
                    vec2f(0., 0.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::BottomLeft,
                    ChildAnchor::BottomLeft,
                ),
            };
            stack.add_positioned_child(
                ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background(appearance.theme().outline())
                        .finish(),
                )
                .with_height(1.)
                .finish(),
                positioning,
            );
        }

        if let Some(sharer) = self.shared_session_sharer() {
            if sharer.is_inactivity_warning_modal_open() {
                stack.add_child(ChildView::new(sharer.inactivity_modal()).finish())
            }
        }

        // Render first-time cloud agent setup view when in Setup status
        if self
            .ambient_agent_view_model
            .as_ref()
            .is_some_and(|model| model.as_ref(app).is_in_setup())
        {
            stack.add_child(ChildView::new(&self.first_time_cloud_agent_setup_view).finish());
        }

        if self.ssh_file_upload.as_ref(app).has_upload() {
            stack.add_child(
                Align::new(ChildView::new(&self.ssh_file_upload).finish())
                    .bottom_right()
                    .finish(),
            );
        }

        let element = if !did_wrap_terminal_size {
            wrap_in_terminal_size_element(
                &self.resize_tx,
                SavePosition::new(stack.finish(), &self.terminal_position_id()).finish(),
            )
        } else {
            SavePosition::new(stack.finish(), &self.terminal_position_id()).finish()
        };

        let final_element = if self.is_file_drop_target && FeatureFlag::SshDragAndDrop.is_enabled()
        {
            Container::new(element)
                .with_foreground_overlay(appearance.theme().accent_overlay())
                .finish()
        } else if FeatureFlag::AgentView.is_enabled()
            && self.agent_view_controller.as_ref(app).is_fullscreen()
        {
            Container::new(element)
                .with_foreground_overlay(agent_view_bg_fill(app))
                .finish()
        } else {
            element
        };

        // Wrap with conversation details panel on the right if open.
        // On WASM, the panel is rendered in the wasm_view instead.
        //
        // Use the `_from_model` variant since `render` already holds
        // `self.model.lock()` and the task-id lookup would otherwise re-lock.
        let should_show_panel = !cfg!(target_family = "wasm")
            && self.is_conversation_details_panel_open
            && self.can_show_conversation_details_ui_from_model(&model, app);

        if should_show_panel {
            // Wrap panel with agent view background for visual consistency
            let panel_with_background =
                Container::new(ChildView::new(&self.conversation_details_panel).finish())
                    .with_background(agent_view_bg_fill(app))
                    .finish();

            Container::new(
                Flex::row()
                    .with_main_axis_size(warpui::elements::MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(Shrinkable::new(1., final_element).finish())
                    .with_child(panel_with_background)
                    .finish(),
            )
            .with_border(Border::top(1.0).with_border_fill(appearance.theme().outline()))
            .finish()
        } else {
            final_element
        }
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.maybe_report_focus_in(ctx);
            ctx.dispatch_typed_action(&PaneGroupAction::HandleFocusChange);

            // Forward focus to the active SSH remote-server choice block so
            // its keyboard-navigable buttons stay interactive.
            if let Some(ssh_choice_view) = self.active_ssh_remote_server_choice_block() {
                ctx.focus(&ssh_choice_view);
            }

            ctx.notify();
        }
        self.update_focused_terminal_info(ctx);
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            // Make sure to close link tooltips when terminal view is not in focus.
            self.open_grid_link_tool_tip.take();

            self.maybe_report_focus_out(ctx);
            ctx.notify();
        }
    }

    fn keymap_context(&self, app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();
        context.map.insert(
            "TerminalView_BlockSelectionCardinality",
            self.selected_blocks.cardinality().as_keymap_context_value(),
        );

        let model_lock = self.model.lock();
        if model_lock.is_block_list_empty() {
            context.set.insert("TerminalView_EmptyBlockList");
        } else {
            context.set.insert("TerminalView_NonEmptyBlockList");
        }

        if self.is_input_box_visible(&model_lock, app) {
            context.set.insert(INPUT_BOX_VISIBLE_KEY);
        }

        if self.input.as_ref(app).editor().as_ref(app).is_focused() {
            context.set.insert("EditorFocused");
        }

        if model_lock.block_list().selection().is_some() {
            context.set.insert("ActiveBlockTextSelection");
        }

        if model_lock.alt_screen().selection().is_some() {
            context.set.insert("ActiveAltScreenSelection");
        }

        if model_lock.is_alt_screen_active() {
            context.set.insert("AltScreen");
        }

        let active_block = model_lock.block_list().active_block();
        if active_block.is_active_and_long_running() {
            if !model_lock.is_alt_screen_active() {
                context.set.insert("LongRunningCommand");
            }

            if active_block.is_agent_monitoring() {
                context
                    .set
                    .insert(LONG_RUNNING_AGENT_REQUESTED_COMMAND_CONTEXT_KEY);

                if active_block.is_eligible_for_agent_handoff() {
                    context
                        .set
                        .insert(LONG_RUNNING_AGENT_REQUESTED_COMMAND_USER_TOOK_OVER_CONTEXT_KEY);
                }
            }
        }

        // Add keyboard protocol context if enabled.
        if model_lock.is_term_mode_set(TermMode::KEYBOARD_PROTOCOL) {
            context.set.insert(init::KEYBOARD_PROTOCOL_ENABLED_KEY);
        }

        if CLIAgentSessionsModel::as_ref(app)
            .session(self.view_id)
            .is_some()
        {
            context.set.insert(init::CLI_AGENT_SESSION_ACTIVE_KEY);
            if *AISettings::as_ref(app).should_render_cli_agent_footer {
                context.set.insert(flags::CLI_AGENT_FOOTER_ENABLED);

                if is_rich_input_chip_in_cli_toolbar(app) {
                    context.set.insert(flags::CLI_AGENT_RICH_INPUT_CHIP_ENABLED);
                }
            }

            // Mirror the rich-input-open flag onto the terminal context so the
            // Ctrl+G toggle binding can close rich input regardless of which
            // descendant view currently holds focus, and even when the
            // active block has transitioned out of `LongRunningCommand`
            // (e.g., the CLI agent has paused waiting for user input). See #9916.
            if CLIAgentSessionsModel::as_ref(app).is_input_open(self.view_id) {
                context.set.insert(flags::CLI_AGENT_RICH_INPUT_OPEN);
            }
        }

        if FeatureFlag::AgentView.is_enabled() {
            context.set.insert(flags::AGENT_VIEW_ENABLED);
            let agent_view_state = self.agent_view_controller.as_ref(app).agent_view_state();
            if agent_view_state.is_fullscreen() {
                context.set.insert(flags::ACTIVE_AGENT_VIEW);
            } else if agent_view_state.is_inline() {
                context.set.insert(flags::ACTIVE_INLINE_AGENT_VIEW);
            }
        }

        if self.is_ambient_agent_session(app) && !self.is_nested_cloud_mode(app) {
            context.set.insert(init::ROOT_CLOUD_MODE_PANE_KEY);
        }

        if let Some(WithinBlockBanner::WarpifyBanner(state)) =
            model_lock.block_list().active_block().block_banner()
        {
            if state.is_ssh() {
                context.set.insert("SshWarpificationBanner");
            } else {
                context.set.insert("SubshellBanner");
            }
        }

        // Also set the warpify context when the footer (flag-gated replacement
        // for the in-block banner) is active, so the ctrl-i keybinding works.
        if let Some(warpify_mode) = self.use_agent_footer.as_ref(app).warpify_mode(app) {
            if warpify_mode.is_ssh() {
                context.set.insert("SshWarpificationBanner");
            } else {
                context.set.insert("SubshellBanner");
            }
        }

        if let Some(SshBlockState::Error { .. }) = self.warpify_state.ssh_block_state() {
            context.set.insert(SSH_ERROR_BLOCK_VISIBLE_KEY);
        }

        if self
            .inline_banners_state
            .prompt_suggestions_banner
            .is_some()
            || has_pending_code_or_unit_test_prompt_suggestion(&model_lock, app)
        {
            context.set.insert(flags::HAS_PENDING_PROMPT_SUGGESTION);
        }

        if AISettings::as_ref(app).is_any_ai_enabled(app) {
            context.set.insert(flags::IS_ANY_AI_ENABLED);
        }

        if self
            .rich_content_views
            .last()
            .and_then(|content| content.metadata())
            .is_some_and(|metadata| {
                matches!(
                    metadata,
                    RichContentMetadata::OnboardingAgenticSuggestions { .. }
                )
            })
            && self.block_onboarding_active
        {
            context.set.insert("OnboardingAgenticSuggestionsBlock");
        }

        if self.current_repo_path.is_some() {
            context.set.insert("InsideRepository");
        }

        #[cfg(not(target_arch = "wasm32"))]
        if self.can_show_conversation_details_ui_from_model(&model_lock, app) {
            context.set.insert(init::CAN_SHOW_CONVERSATION_DETAILS_KEY);
        }

        let active_conversation = if FeatureFlag::AgentView.is_enabled() {
            self.agent_view_controller
                .as_ref(app)
                .agent_view_state()
                .active_conversation_id()
                .and_then(|id| BlocklistAIHistoryModel::as_ref(app).conversation(&id))
        } else {
            BlocklistAIHistoryModel::as_ref(app).active_conversation(self.id())
        };
        // Set CanResumeConversation flag if the latest exchange (across all tasks,
        // including subtasks) was manually cancelled or finished with an error.
        if FeatureFlag::AIResumeButton.is_enabled() {
            let latest_exchange = active_conversation.and_then(|c| c.latest_exchange());
            let was_manually_cancelled = latest_exchange
                .and_then(|e| e.output_status.cancel_reason())
                .is_some_and(|reason| reason.is_manually_cancelled());
            let has_error = active_conversation.is_some_and(|c| c.status().is_error());
            if was_manually_cancelled || has_error {
                context.set.insert(init::CAN_RESUME_CONVERSATION_KEY);
            }
        }
        if active_conversation
            .as_ref()
            .and_then(|conversation| {
                fork_from_last_known_good_state_exchange_id(conversation, &model_lock)
            })
            .is_some()
        {
            context
                .set
                .insert(init::CAN_FORK_FROM_LAST_KNOWN_GOOD_STATE_KEY);
        }

        context
            .set
            .insert(model_lock.shared_session_status().as_keymap_context());

        #[cfg(feature = "local_fs")]
        {
            let imported_config_model = ImportedConfigModel::as_ref(app);
            if !imported_config_model.finished_searching_for_settings()
                || imported_config_model.configs().count() >= 1
            {
                context.set.insert(flags::HAS_SETTINGS_TO_IMPORT_FLAG);
            }
        }

        context
    }

    fn active_cursor_position(&self, ctx: &ViewContext<Self>) -> Option<CursorInfo> {
        let cursor_id = self.cursor_position_id();
        let appearance = Appearance::as_ref(ctx);
        let font_size = appearance.monospace_font_size();

        ctx.element_position_by_id(cursor_id)
            .map(|position| CursorInfo {
                position,
                font_size,
            })
    }

    fn self_or_child_interacted_with(&self, _ctx: &mut ViewContext<Self>) {
        if let Some(sharer) = self.shared_session_sharer() {
            // If warning modal is open, sharer must continue share through the modal
            if !sharer.is_inactivity_warning_modal_open() {
                if let Err(e) = sharer.activity_tx().try_send(()) {
                    log::warn!("Failed to send sharer activity over activity_tx channel {e:?}");
                }
            }
        }
    }

    fn accessibility_data(&self, ctx: &mut ViewContext<Self>) -> Option<AccessibilityData> {
        const PER_BLOCK_LINE_LIMIT: usize = 5000;

        let terminal_session_content = if self.model.lock().is_alt_screen_active() {
            self.model.lock().alt_screen().output_to_string()
        } else {
            let last_five_blocks_content = {
                let model = self.model.lock();
                let agent_view_state = model.block_list().agent_view_state();
                let blocks = model
                    .block_list()
                    .blocks()
                    .iter()
                    .filter(|block| block.is_visible(agent_view_state))
                    .rev()
                    .take(5)
                    .collect_vec();

                // Produce a final string of the contents of each block, followed by the input.
                blocks
                    .iter()
                    .rev()
                    .map(|block| block.contents_to_string_with_line_limit(PER_BLOCK_LINE_LIMIT))
                    .collect_vec()
            };

            if self.is_input_box_visible(&self.model.lock(), ctx) {
                let (prompt_text, _rprompt) = self.input.as_ref(ctx).prompt_and_rprompt_text(ctx);
                let input_text = self.input.as_ref(ctx).buffer_text(ctx);
                last_five_blocks_content
                    .into_iter()
                    .chain([format!("{prompt_text} {input_text}")])
                    .join("\n")
            } else {
                last_five_blocks_content.join("\n")
            }
        };

        Some(AccessibilityData {
            content: terminal_session_content,
        })
    }
}

/// Readable summary for an AI block.
struct AIBlockNotificationSummary {
    title: String,
    description: String,
    success: bool,
}

/// A menu positioning provider for when the input is rendered within the terminal.
struct TerminalViewMenuPositioningProvider {
    parent: WeakViewHandle<TerminalView>,
}

impl MenuPositioningProvider for TerminalViewMenuPositioningProvider {
    fn menu_position(&self, app: &AppContext) -> MenuPositioning {
        if let Some(terminal_view) = self.parent.upgrade(app) {
            let view_ref = terminal_view.as_ref(app);
            let TerminalView {
                model, size_info, ..
            } = view_ref;
            let model = model.lock();
            let input_mode = if view_ref.agent_view_controller.as_ref(app).is_fullscreen() {
                InputMode::PinnedToBottom
            } else {
                *InputModeSettings::as_ref(app).input_mode.value()
            };
            let total_block_height_px = (model.block_list().block_heights().summary().height)
                .to_pixels(size_info.cell_height_px);

            // Menus are positioned as follows:
            // - Always above the input for pinned to bottom
            // - Always below the input for pinned to top
            // - In Waterfall mode with no-gap, conditionally above or below depending
            //   on whether the blocks take up more or less than half of the viewport size.
            // - In Waterfall mode with a gap, conditionally above or below depending
            //   on the size of the gap and the scroll position.  Basically, if the input is
            //   is more than halfway down the screen, the menus render above; otherwise they render below.
            let positioning = match (input_mode, model.block_list().active_gap()) {
                (InputMode::PinnedToBottom, _) => MenuPositioning::AboveInputBox,
                (InputMode::PinnedToTop, _) => MenuPositioning::BelowInputBox,
                (InputMode::Waterfall, None) => {
                    let height_ratio =
                        total_block_height_px.as_f32() / size_info.pane_height_px().as_f32();
                    if height_ratio < 0.5 {
                        MenuPositioning::BelowInputBox
                    } else {
                        MenuPositioning::AboveInputBox
                    }
                }
                (InputMode::Waterfall, Some(gap)) => {
                    let viewport = view_ref.viewport_state(model.block_list(), input_mode, app);
                    let scroll_top_px = viewport
                        .scroll_top_in_lines()
                        .to_pixels(size_info.cell_height_px());
                    // Calculate how far into the viewport the input is (in pixels from the top).
                    let input_position_in_viewport_px = total_block_height_px
                        - gap.height().to_pixels(size_info.cell_height_px())
                        - scroll_top_px;
                    let height_ratio = input_position_in_viewport_px.as_f32()
                        / size_info.pane_height_px().as_f32();
                    if height_ratio < 0.5 {
                        MenuPositioning::BelowInputBox
                    } else {
                        MenuPositioning::AboveInputBox
                    }
                }
            };
            return positioning;
        }

        // If we can't upgrade the terminal view reference, fall back to using just the InputMode setting
        let input_mode = *InputModeSettings::as_ref(app).input_mode.value();

        match input_mode {
            InputMode::PinnedToBottom => MenuPositioning::AboveInputBox,
            InputMode::PinnedToTop => MenuPositioning::BelowInputBox,
            InputMode::Waterfall => {
                // For Waterfall mode without terminal view context, default to BelowInputBox
                MenuPositioning::BelowInputBox
            }
        }
    }

    fn inline_menu_position(&self, inline_menu_height: f32, app: &AppContext) -> MenuPositioning {
        let Some(terminal_view) = self.parent.upgrade(app) else {
            return MenuPositioning::AboveInputBox;
        };

        let terminal_content_height = terminal_view.as_ref(app).content_element_height_px(app);
        if terminal_content_height > inline_menu_height {
            MenuPositioning::AboveInputBox
        } else {
            MenuPositioning::BelowInputBox
        }
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        if let Some((is_bootstrapped, pending_shell, has_pending_ssh_session)) =
            self.model.try_lock().map(|model| {
                (
                    model.block_list().is_bootstrapped(),
                    model.pending_shell_type(),
                    model.has_pending_ssh_session(),
                )
            })
        {
            if has_pending_ssh_session || !is_bootstrapped {
                // Only treat session abandonment as an error if the session was
                // visible to the user at some point.  This filters out
                // bootstrap "failures" such as oh-my-zsh prompting the user
                // about an update while we're sourcing their rcfiles - we'll
                // never technically finish bootstrapping the shell.  If that
                // occurs in some non-visible tab, we don't want to conflate it
                // (an unanswered prompt) with an actual failure to bootstrap
                // the shell.
                let log_level = if self.was_ever_visible {
                    log::Level::Error
                } else {
                    log::Level::Warn
                };
                log::log!(
                    log_level,
                    "Session abandoned before bootstrap for shell {pending_shell:?} on ssh {has_pending_ssh_session}"
                );

                let was_ever_visible = self.was_ever_visible;
                let duration_since_start =
                    self.bootstrap_start.unwrap_or_else(Instant::now).elapsed();
                let server_api = self.server_api.clone();
                let privacy_settings_snapshot = self.privacy_settings_snapshot;
                let task = self.background_executor.spawn(async move {
                    if let Err(error) = server_api
                        .send_telemetry_event(
                            TelemetryEvent::SessionAbandonedBeforeBootstrap {
                                pending_shell,
                                has_pending_ssh_session,
                                was_ever_visible,
                                duration_since_start,
                            },
                            privacy_settings_snapshot,
                        )
                        .await
                    {
                        log::warn!("Error occurred with sending telemetry event: {error}");
                    }
                });
                task.detach();
            }
        };
    }
}

/// Returns an instance of [`SizeInfo`] that is to be used
/// when in the blocklist.
///
/// This should really only be used when it's only possible to be
/// in the blocklist (e.g. starting a session / creating a [`TerminalModel`]).
/// Otherwise, use [`create_size_info`].
pub fn create_size_info_for_blocklist(
    pane_size: Vector2F,
    font_cache: &FontCache,
    font_family_id: FamilyId,
    font_size: f32,
    line_height_ratio: f32,
) -> SizeInfo {
    let cell_size_px =
        grid_cell_dimensions(font_cache, font_family_id, font_size, line_height_ratio);

    // Note: `SizeInfo` treats the padding as symmetric, so for bottom-only padding we divide by 2
    let padding_x = PADDING_LEFT.into_pixels();
    let padding_y = (cell_size_px.y() * LONG_RUNNING_BOTTOM_PADDING_LINES / 2.).into_pixels();

    SizeInfo::new(
        pane_size,
        cell_size_px.x().into_pixels(),
        cell_size_px.y().into_pixels(),
        padding_x,
        padding_y,
    )
}

/// Returns an instance of [`SizeInfo`] that accounts for the current
/// terminal mode (alt-screen vs. blocklist).
#[allow(clippy::too_many_arguments)]
pub fn create_size_info(
    pane_size: Vector2F,
    model: &TerminalModel,
    sessions: &Sessions,
    font_cache: &FontCache,
    font_family_id: FamilyId,
    font_size: f32,
    line_height_ratio: f32,
    ctx: &AppContext,
) -> SizeInfo {
    let cell_size_px =
        grid_cell_dimensions(font_cache, font_family_id, font_size, line_height_ratio);
    let active_command = model
        .block_list()
        .active_block()
        .top_level_command(sessions);

    match *TerminalSettings::as_ref(ctx).alt_screen_padding {
        AltScreenPaddingMode::Custom { uniform_padding }
            if model.is_alt_screen_active()
                // If we don't know what the top-level command is,
                // it's not denylisted so we use the custom padding.
                && active_command.is_none_or(|cmd| {
                    !ALT_SCREEN_APPS_THAT_MUST_MATCH_BLOCKLIST_PADDING.contains(cmd.as_str())
                }) =>
        {
            SizeInfo::new(
                pane_size,
                cell_size_px.x().into_pixels(),
                cell_size_px.y().into_pixels(),
                uniform_padding,
                uniform_padding,
            )
        }
        _ => create_size_info_for_blocklist(
            pane_size,
            font_cache,
            font_family_id,
            font_size,
            line_height_ratio,
        ),
    }
}

/// Returns CellSizeAndWindowPadding for the given font params and line height.
pub fn cell_size_and_padding(
    font_cache: &FontCache,
    font_family_id: FamilyId,
    font_size: f32,
    line_height_ratio: f32,
) -> CellSizeAndWindowPadding {
    let cell_size_px =
        grid_cell_dimensions(font_cache, font_family_id, font_size, line_height_ratio);
    let (padding_x_px, padding_y_px) = (
        *PADDING_LEFT,
        cell_size_px.y() * LONG_RUNNING_BOTTOM_PADDING_LINES / 2.,
    );

    CellSizeAndWindowPadding {
        cell_width_px: cell_size_px.x().into_pixels(),
        cell_height_px: cell_size_px.y().into_pixels(),
        padding_x_px: padding_x_px.into_pixels(),
        padding_y_px: padding_y_px.into_pixels(),
    }
}

fn command_first_word_and_suffix(command: &str) -> Option<(&str, &str)> {
    let first_word = command.split_whitespace().next()?;
    let word_start = command.find(first_word)?;
    let rest = &command[word_start + first_word.len()..];
    Some((first_word, rest))
}

/// Conditionally wrap a terminal element (altscreen / blocklist element) in a scrollable element.
/// TODO: We should not conditionally composite the scrollable element.
#[allow(clippy::too_many_arguments)]
fn maybe_wrap_terminal_element_in_scrollable(
    is_scrollable_vertical: bool,
    is_scrollable_horizontal: bool,
    vertical_scroll_handle: ScrollStateHandle,
    horizontal_scroll_handle: ClippedScrollStateHandle,
    required_terminal_width: f32,
    theme: &WarpTheme,
    element: impl NewScrollableElement + 'static,
) -> Box<dyn Element> {
    let nonactive_thumb_background = theme.disabled_text_color(theme.background()).into();
    let active_thumb_background = theme.main_text_color(theme.background()).into();
    let track_background = Fill::None;
    let scrollbar_appearance = ScrollableAppearance::new(SCROLLBAR_WIDTH, true);
    match (is_scrollable_vertical, is_scrollable_horizontal) {
        (true, true) => {
            let config = DualAxisConfig::Manual {
                horizontal: AxisConfiguration::Clipped(ClippedAxisConfiguration {
                    handle: horizontal_scroll_handle,
                    max_size: Some(required_terminal_width),
                    stretch_child: false,
                }),
                vertical: AxisConfiguration::Manual(vertical_scroll_handle),
                child: NewScrollableElement::finish_scrollable(element),
            };

            NewScrollable::horizontal_and_vertical(
                config,
                nonactive_thumb_background,
                active_thumb_background,
                track_background,
            )
            .with_horizontal_scrollbar(scrollbar_appearance)
            .with_vertical_scrollbar(scrollbar_appearance)
            .finish()
        }
        (true, false) => {
            let config = SingleAxisConfig::Manual {
                handle: vertical_scroll_handle,
                child: NewScrollableElement::finish_scrollable(element),
            };
            NewScrollable::vertical(
                config,
                nonactive_thumb_background,
                active_thumb_background,
                track_background,
            )
            .with_vertical_scrollbar(scrollbar_appearance)
            .finish()
        }
        (false, true) => {
            let config = SingleAxisConfig::Clipped {
                handle: horizontal_scroll_handle,
                child: ConstrainedBox::new(element.finish())
                    .with_max_width(required_terminal_width)
                    .finish(),
            };
            NewScrollable::horizontal(
                config,
                nonactive_thumb_background,
                active_thumb_background,
                track_background,
            )
            .with_horizontal_scrollbar(scrollbar_appearance)
            .finish()
        }
        (false, false) => element.finish(),
    }
}

/// Returns `true` when the Rich Input chip is present in the user's CLI agent
/// footer toolbar configuration.
fn is_rich_input_chip_in_cli_toolbar(app: &AppContext) -> bool {
    let sel = &SessionSettings::as_ref(app).cli_agent_footer_chip_selection;
    sel.left_items()
        .iter()
        .chain(sel.right_items().iter())
        .any(|item| matches!(item, AgentToolbarItemKind::RichInput))
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
