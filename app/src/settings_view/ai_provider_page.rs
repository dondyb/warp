//! Settings page for configuring a custom AI provider (OpenAI- or
//! Anthropic-compatible endpoint). Replaces the env-var-only path
//! from M1b-chat.
//!
//! Form fields and local state live here. Persistence is added in Tasks 7 + 8.
//! Test-connection wiring is added in Task 9.

use super::{
    settings_page::{
        build_sub_header, render_separator, MatchData, PageType, SettingsPageEvent,
        SettingsPageMeta, SettingsPageViewHandle, SettingsWidget, CONTENT_FONT_SIZE,
        HEADER_PADDING,
    },
    SettingsSection,
};
use crate::appearance::Appearance;
use crate::editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextColors, TextOptions};
use crate::settings::ai_provider::AiProviderSettings;
use settings::Setting as _;
use crate::view_components::{
    action_button::{ActionButton, ButtonSize, SecondaryTheme},
    dropdown::{Dropdown, DropdownItem},
};
use warpui::{
    elements::{
        ChildView, Container, CrossAxisAlignment, Element, Expanded, Flex, MainAxisAlignment,
        MainAxisSize, ParentElement, Text,
    },
    ui_components::components::{Coords, UiComponent, UiComponentStyles},
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};
use warpui::r#async::SpawnedFutureHandle;
use warpui_extras::secure_storage::AppContextExt as _;

/// Secure-storage key for the user's BYO LLM API key. Distinct from
/// `crates/ai/src/api_keys.rs`'s `"AiApiKeys"` (which stores Warp's
/// hosted-AI keys) so the two don't collide.
const API_KEY_STORAGE_KEY: &str = "BringYourOwnLlmApiKey";

fn load_byo_api_key(ctx: &warpui::AppContext) -> Option<String> {
    ctx.secure_storage().read_value(API_KEY_STORAGE_KEY).ok()
}

fn save_byo_api_key(ctx: &warpui::AppContext, key: &str) {
    if let Err(e) = ctx.secure_storage().write_value(API_KEY_STORAGE_KEY, key) {
        log::warn!("failed to save BYO LLM API key to secure storage: {e:#}");
    }
}

/// Read the current settings + secure storage and push them into the
/// process-wide `ai_provider::RUNTIME_CONFIG` singleton so that the
/// dispatcher picks up GUI-configured values without requiring env vars.
fn push_runtime_config_from_settings(ctx: &warpui::AppContext) {
    let settings = AiProviderSettings::as_ref(ctx);
    let endpoint = settings.endpoint.value().to_string();
    let model = settings.model.value().to_string();
    let api_key = ctx
        .secure_storage()
        .read_value(API_KEY_STORAGE_KEY)
        .unwrap_or_default();

    let cfg = ai_provider::OpenAiConfig::from_parts(endpoint, api_key, model).ok();
    ai_provider::set_runtime_config(cfg);
}

// ── Page action ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum AiProviderPageAction {
    /// User changed the protocol selection.
    SelectProtocol(AiProtocol),
    /// "Test connection" button clicked — wired up in Task 9.
    TestConnection,
}

// ── Protocol enum ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiProtocol {
    OpenAi,
    Anthropic,
}

impl std::fmt::Display for AiProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiProtocol::OpenAi => write!(f, "OpenAI"),
            AiProtocol::Anthropic => write!(f, "Anthropic"),
        }
    }
}

// ── Test-connection status ────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub(crate) enum TestStatus {
    #[default]
    Idle,
    InProgress,
    Success,
    Failure(String),
}

// ── Widget ────────────────────────────────────────────────────────────────────

struct AiProviderConfigWidget {
    endpoint_editor: ViewHandle<EditorView>,
    api_key_editor: ViewHandle<EditorView>,
    model_editor: ViewHandle<EditorView>,
    protocol_dropdown: ViewHandle<Dropdown<AiProviderPageAction>>,
    test_button: ViewHandle<ActionButton>,
}

impl AiProviderConfigWidget {
    fn new(ctx: &mut ViewContext<AiProviderPageView>) -> Self {
        let appearance = Appearance::as_ref(ctx);
        let font_size = appearance.ui_font_size();
        let monospace_family = appearance.monospace_font_family();
        let active_color = appearance.theme().active_ui_text_color();
        let disabled_color = appearance.theme().disabled_ui_text_color();

        let make_editor_options = |is_password: bool| SingleLineEditorOptions {
            is_password,
            text: TextOptions {
                font_size_override: Some(font_size),
                font_family_override: Some(monospace_family),
                text_colors_override: Some(TextColors {
                    default_color: active_color,
                    disabled_color,
                    hint_color: disabled_color,
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        // Load saved values from settings.
        let saved_endpoint = AiProviderSettings::as_ref(ctx).endpoint.value().clone();
        let saved_model = AiProviderSettings::as_ref(ctx).model.value().clone();
        let saved_protocol = AiProviderSettings::as_ref(ctx).protocol.value().clone();

        let endpoint_editor = ctx.add_typed_action_view(move |ctx| {
            let mut editor = EditorView::single_line(make_editor_options(false), ctx);
            editor.set_placeholder_text("https://api.openai.com/v1", ctx);
            if !saved_endpoint.is_empty() {
                editor.set_buffer_text(&saved_endpoint, ctx);
            }
            editor
        });

        let saved_key = load_byo_api_key(ctx).unwrap_or_default();
        let api_key_editor = ctx.add_typed_action_view(move |ctx| {
            let mut editor = EditorView::single_line(make_editor_options(true), ctx);
            editor.set_placeholder_text("sk-...", ctx);
            if !saved_key.is_empty() {
                editor.set_buffer_text(&saved_key, ctx);
            }
            editor
        });

        let saved_model_for_editor = saved_model.clone();
        let model_editor = ctx.add_typed_action_view(move |ctx| {
            let mut editor = EditorView::single_line(make_editor_options(false), ctx);
            editor.set_placeholder_text("gpt-4o", ctx);
            if !saved_model_for_editor.is_empty() {
                editor.set_buffer_text(&saved_model_for_editor, ctx);
            }
            editor
        });

        let initial_protocol_index = if saved_protocol == "anthropic" { 1 } else { 0 };
        let protocol_dropdown = ctx.add_typed_action_view(move |ctx| {
            let mut dropdown = Dropdown::new(ctx);
            let items = vec![
                DropdownItem::new("OpenAI", AiProviderPageAction::SelectProtocol(AiProtocol::OpenAi)),
                DropdownItem::new(
                    "Anthropic",
                    AiProviderPageAction::SelectProtocol(AiProtocol::Anthropic),
                ),
            ];
            dropdown.add_items(items, ctx);
            dropdown.set_selected_by_index(initial_protocol_index, ctx);
            dropdown
        });

        // Save endpoint on every edit.
        ctx.subscribe_to_view(&endpoint_editor, |_me, editor_handle, event, ctx| {
            if let EditorEvent::Edited(_) = event {
                let text = editor_handle.as_ref(ctx).buffer_text(ctx);
                AiProviderSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings.endpoint.set_value(text, ctx);
                });
                push_runtime_config_from_settings(ctx);
            }
        });

        // Save model on every edit.
        ctx.subscribe_to_view(&model_editor, |_me, editor_handle, event, ctx| {
            if let EditorEvent::Edited(_) = event {
                let text = editor_handle.as_ref(ctx).buffer_text(ctx);
                AiProviderSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings.model.set_value(text, ctx);
                });
                push_runtime_config_from_settings(ctx);
            }
        });

        // Save API key on every edit (secure storage, not TOML).
        ctx.subscribe_to_view(&api_key_editor, |_me, editor_handle, event, ctx| {
            if let EditorEvent::Edited(_) = event {
                let text = editor_handle.as_ref(ctx).buffer_text(ctx);
                save_byo_api_key(ctx, &text);
                push_runtime_config_from_settings(ctx);
            }
        });

        let test_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Test connection", SecondaryTheme)
                .with_size(ButtonSize::Default)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AiProviderPageAction::TestConnection);
                })
        });

        // Prime the runtime config from saved settings so the dispatcher
        // picks up GUI values even before the user edits anything this session.
        push_runtime_config_from_settings(ctx);

        Self {
            endpoint_editor,
            api_key_editor,
            model_editor,
            protocol_dropdown,
            test_button,
        }
    }
}

impl SettingsWidget for AiProviderConfigWidget {
    type View = AiProviderPageView;

    fn search_terms(&self) -> &str {
        "ai provider custom endpoint openai anthropic api key model protocol test connection"
    }

    fn render(
        &self,
        view: &AiProviderPageView,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let label_color = theme.active_ui_text_color();

        let editor_style = UiComponentStyles {
            padding: Some(Coords {
                top: 10.,
                bottom: 10.,
                left: 16.,
                right: 16.,
            }),
            background: Some(theme.surface_2().into()),
            ..Default::default()
        };

        // Helper: render a label + editor pair stacked vertically.
        let render_field = |label: &str, editor: &ViewHandle<EditorView>| -> Box<dyn Element> {
            let label_el = Text::new_inline(
                label.to_string(),
                appearance.ui_font_family(),
                CONTENT_FONT_SIZE,
            )
            .with_color(label_color.into())
            .finish();

            let input = appearance
                .ui_builder()
                .text_input(editor.clone())
                .with_style(editor_style)
                .build()
                .finish();

            Flex::column()
                .with_spacing(8.)
                .with_child(label_el)
                .with_child(input)
                .finish()
        };

        // Protocol dropdown row (label left, dropdown right).
        let protocol_label = Text::new_inline(
            "Protocol".to_string(),
            appearance.ui_font_family(),
            CONTENT_FONT_SIZE,
        )
        .with_color(label_color.into())
        .finish();

        let protocol_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(Expanded::new(1., protocol_label).finish())
            .with_child(ChildView::new(&self.protocol_dropdown).finish())
            .finish();

        // Test button row with status text.
        let test_button_el = ChildView::new(&self.test_button).finish();
        let test_status_el: Box<dyn Element> = match &view.test_status {
            TestStatus::Idle => warpui::elements::Empty::new().finish(),
            TestStatus::InProgress => Text::new_inline(
                "Testing\u{2026}".to_string(),
                appearance.ui_font_family(),
                CONTENT_FONT_SIZE,
            )
            .with_color(theme.disabled_ui_text_color().into())
            .finish(),
            TestStatus::Success => Text::new_inline(
                "\u{2713} Connection OK".to_string(),
                appearance.ui_font_family(),
                CONTENT_FONT_SIZE,
            )
            .with_color(theme.ui_green_color())
            .finish(),
            TestStatus::Failure(msg) => Text::new_inline(
                format!("\u{2717} {msg}"),
                appearance.ui_font_family(),
                CONTENT_FONT_SIZE,
            )
            .with_color(theme.ui_error_color())
            .finish(),
        };

        let test_row = Flex::row()
            .with_spacing(12.)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(test_button_el)
            .with_child(test_status_el)
            .finish();

        // Assemble form.
        let column = Flex::column()
            .with_spacing(16.)
            .with_child(render_separator(appearance))
            .with_child(
                build_sub_header(appearance, "Custom AI Provider", None)
                    .with_padding_bottom(HEADER_PADDING)
                    .finish(),
            )
            .with_child(render_field("Endpoint URL", &self.endpoint_editor))
            .with_child(render_field("API Key", &self.api_key_editor))
            .with_child(render_field("Model", &self.model_editor))
            .with_child(
                Container::new(protocol_row)
                    .with_padding_top(4.)
                    .with_padding_bottom(4.)
                    .finish(),
            )
            .with_child(test_row)
            .finish();

        Container::new(column)
            .with_margin_bottom(HEADER_PADDING)
            .finish()
    }
}

// ── Page view ─────────────────────────────────────────────────────────────────

pub struct AiProviderPageView {
    page: PageType<Self>,
    /// Cloned handles to the live editor views so `handle_action` can read
    /// their current buffer text without going through saved settings.
    endpoint_editor: ViewHandle<EditorView>,
    api_key_editor: ViewHandle<EditorView>,
    model_editor: ViewHandle<EditorView>,
    /// Current state of the "Test connection" operation.
    pub(crate) test_status: TestStatus,
    /// Handle to the in-flight test future, kept so it can be aborted
    /// if the user clicks Test again before the previous one finishes.
    _test_future: Option<SpawnedFutureHandle>,
}

impl AiProviderPageView {
    pub fn new(ctx: &mut ViewContext<AiProviderPageView>) -> Self {
        let widget = AiProviderConfigWidget::new(ctx);
        // Clone handles before the widget is moved into the page.
        let endpoint_editor = widget.endpoint_editor.clone();
        let api_key_editor = widget.api_key_editor.clone();
        let model_editor = widget.model_editor.clone();
        AiProviderPageView {
            page: PageType::new_monolith(widget, None, false),
            endpoint_editor,
            api_key_editor,
            model_editor,
            test_status: TestStatus::Idle,
            _test_future: None,
        }
    }
}

impl Entity for AiProviderPageView {
    type Event = SettingsPageEvent;
}

impl View for AiProviderPageView {
    fn ui_name() -> &'static str {
        "AiProviderPage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl TypedActionView for AiProviderPageView {
    type Action = AiProviderPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AiProviderPageAction::SelectProtocol(protocol) => {
                let protocol_str = match protocol {
                    AiProtocol::OpenAi => "openai".to_string(),
                    AiProtocol::Anthropic => "anthropic".to_string(),
                };
                AiProviderSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings.protocol.set_value(protocol_str, ctx);
                });
                push_runtime_config_from_settings(ctx);
            }
            AiProviderPageAction::TestConnection => {
                log::info!(
                    "[ai_provider_page] Test connection clicked. \
                     Reading endpoint/model/api_key from live editors."
                );
                // Read from the LIVE editor buffers so we always test what
                // the user has typed RIGHT NOW, even if save-on-edit hasn't
                // fired yet.
                let endpoint = self.endpoint_editor.as_ref(ctx).buffer_text(ctx);
                let api_key = self.api_key_editor.as_ref(ctx).buffer_text(ctx);
                let model = self.model_editor.as_ref(ctx).buffer_text(ctx);

                // Validate with from_parts before kicking off the async task.
                let config = match ai_provider::OpenAiConfig::from_parts(endpoint, api_key, model) {
                    Ok(c) => c,
                    Err(e) => {
                        self.test_status = TestStatus::Failure(format!("{e:#}"));
                        ctx.notify();
                        return;
                    }
                };

                self.test_status = TestStatus::InProgress;
                ctx.notify();

                // Build a minimal "ping" request.
                let request = build_ping_request();

                // Abort any in-flight test.
                self._test_future = None;

                use warpui::r#async::FutureExt as _;
                let handle = ctx.spawn(
                    async move {
                        let adapter = ai_provider::OpenAiAdapter::new(config);
                        log::info!(
                            "[ai_provider_page] Test connection: opening stream to endpoint"
                        );
                        use ai_provider::AiProvider as _;
                        use futures::StreamExt as _;
                        let outcome = adapter
                            .chat_stream(&request)
                            .with_timeout(std::time::Duration::from_secs(10))
                            .await;
                        match outcome {
                            Ok(Ok(mut stream)) => {
                                // Consume up to a few events. If the first non-StreamInit
                                // event is an Error, surface that. If we get past the
                                // opening events without an error, treat the connection
                                // as healthy.
                                let mut events_seen = 0;
                                loop {
                                    let next = stream
                                        .next()
                                        .with_timeout(std::time::Duration::from_secs(10))
                                        .await;
                                    match next {
                                        Ok(Some(Ok(_event))) => {
                                            events_seen += 1;
                                            // Any 3 events without an error → success.
                                            // The first is StreamInit (synthesized), the
                                            // next two are the opening ClientActions.
                                            if events_seen >= 3 {
                                                break Ok(());
                                            }
                                        }
                                        Ok(Some(Err(e))) => break Err(format!("{e:#}")),
                                        Ok(None) => break Err(
                                            "Endpoint closed connection without responding"
                                                .into(),
                                        ),
                                        Err(_timeout) => break Err("Connection timed out".into()),
                                    }
                                }
                            }
                            Ok(Err(e)) => Err(format!("{e:#}")),
                            Err(_timeout) => Err("Connection timed out".to_string()),
                        }
                    },
                    |me, result, ctx| {
                        me.test_status = match result {
                            Ok(()) => {
                                // Push the just-validated values into the runtime config
                                // so the dispatcher uses them immediately.
                                push_runtime_config_from_settings(ctx);
                                TestStatus::Success
                            }
                            Err(msg) => TestStatus::Failure(msg),
                        };
                        me._test_future = None;
                        ctx.notify();
                    },
                );
                self._test_future = Some(handle);
            }
        }
    }
}

/// Build a minimal one-shot "ping" request to test that the endpoint responds.
fn build_ping_request() -> warp_multi_agent_api::Request {
    use warp_multi_agent_api::request as req;
    warp_multi_agent_api::Request {
        input: Some(req::Input {
            r#type: Some(req::input::Type::UserInputs(req::input::UserInputs {
                inputs: vec![req::input::user_inputs::UserInput {
                    input: Some(
                        req::input::user_inputs::user_input::Input::UserQuery(
                            req::input::UserQuery {
                                query: "ping".to_string(),
                                ..Default::default()
                            },
                        ),
                    ),
                }],
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}

impl SettingsPageMeta for AiProviderPageView {
    fn section() -> SettingsSection {
        SettingsSection::AiProvider
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<AiProviderPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<AiProviderPageView>) -> Self {
        SettingsPageViewHandle::AiProvider(view_handle)
    }
}
