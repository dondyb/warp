//! Settings page for configuring a custom AI provider (OpenAI- or
//! Anthropic-compatible endpoint). Replaces the env-var-only path
//! from M1b-chat. Form fields, persistence, and model-fetch wiring
//! all live here.

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

/// Read the current settings and push them into the process-wide
/// `ai_provider::RUNTIME_CONFIG` singleton so the dispatcher picks up
/// GUI-configured values without requiring env vars.
fn push_runtime_config_from_settings(ctx: &warpui::AppContext) {
    let settings = AiProviderSettings::as_ref(ctx);
    let endpoint = settings.endpoint.value().to_string();
    let model = settings.model.value().to_string();
    let api_key = settings.api_key.value().to_string();

    let cfg = ai_provider::OpenAiConfig::from_parts(endpoint, api_key, model).ok();
    ai_provider::set_runtime_config(cfg);
}

// ── Page action ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum AiProviderPageAction {
    /// User changed the protocol selection.
    SelectProtocol(AiProtocol),
    /// User selected a model from the dropdown.
    SelectModel(String),
    /// "Connect" button clicked — fetch /v1/models and populate the dropdown.
    Connect,
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
    Success(usize),
    Failure(String),
}

// ── Widget ────────────────────────────────────────────────────────────────────

struct AiProviderConfigWidget {
    endpoint_editor: ViewHandle<EditorView>,
    api_key_editor: ViewHandle<EditorView>,
    model_dropdown: ViewHandle<Dropdown<AiProviderPageAction>>,
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

        let saved_key = AiProviderSettings::as_ref(ctx).api_key.value().clone();
        let api_key_editor = ctx.add_typed_action_view(move |ctx| {
            let mut editor = EditorView::single_line(make_editor_options(true), ctx);
            editor.set_placeholder_text("sk-...", ctx);
            if !saved_key.is_empty() {
                editor.set_buffer_text(&saved_key, ctx);
            }
            editor
        });

        // Initialize the model dropdown with the saved model (if any) as
        // its only entry. The user clicks "Connect" to replace this with the
        // live list from `/v1/models`.
        let saved_model_for_dropdown = saved_model.clone();
        let model_dropdown = ctx.add_typed_action_view(move |ctx| {
            let mut dropdown = Dropdown::new(ctx);
            if !saved_model_for_dropdown.is_empty() {
                let items = vec![DropdownItem::new(
                    saved_model_for_dropdown.clone(),
                    AiProviderPageAction::SelectModel(saved_model_for_dropdown.clone()),
                )];
                dropdown.add_items(items, ctx);
                dropdown.set_selected_by_index(0, ctx);
            }
            dropdown
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

        // Save API key on every edit.
        ctx.subscribe_to_view(&api_key_editor, |_me, editor_handle, event, ctx| {
            if let EditorEvent::Edited(_) = event {
                let text = editor_handle.as_ref(ctx).buffer_text(ctx);
                AiProviderSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings.api_key.set_value(text, ctx);
                });
                push_runtime_config_from_settings(ctx);
            }
        });

        let test_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Connect", SecondaryTheme)
                .with_size(ButtonSize::Default)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AiProviderPageAction::Connect);
                })
        });

        // Prime the runtime config from saved settings so the dispatcher
        // picks up GUI values even before the user edits anything this session.
        push_runtime_config_from_settings(ctx);

        Self {
            endpoint_editor,
            api_key_editor,
            model_dropdown,
            protocol_dropdown,
            test_button,
        }
    }
}

impl SettingsWidget for AiProviderConfigWidget {
    type View = AiProviderPageView;

    fn search_terms(&self) -> &str {
        "ai provider custom endpoint openai anthropic api key model protocol connect"
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

        // Model dropdown row (label left, dropdown right).
        let model_label = Text::new_inline(
            "Model".to_string(),
            appearance.ui_font_family(),
            CONTENT_FONT_SIZE,
        )
        .with_color(label_color.into())
        .finish();

        let model_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_child(Expanded::new(1., model_label).finish())
            .with_child(ChildView::new(&self.model_dropdown).finish())
            .finish();

        // Connect button row with status text.
        let test_button_el = ChildView::new(&self.test_button).finish();
        let test_status_el: Box<dyn Element> = match &view.test_status {
            TestStatus::Idle => warpui::elements::Empty::new().finish(),
            TestStatus::InProgress => Text::new_inline(
                "Connecting\u{2026}".to_string(),
                appearance.ui_font_family(),
                CONTENT_FONT_SIZE,
            )
            .with_color(theme.disabled_ui_text_color().into())
            .finish(),
            TestStatus::Success(count) => Text::new_inline(
                format!("\u{2713} Found {count} models"),
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
            .with_child(
                Container::new(model_row)
                    .with_padding_top(4.)
                    .with_padding_bottom(4.)
                    .finish(),
            )
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
    model_dropdown: ViewHandle<Dropdown<AiProviderPageAction>>,
    /// Current state of the "Connect" operation.
    pub(crate) test_status: TestStatus,
    /// Handle to the in-flight test future, kept so it can be aborted
    /// if the user clicks Connect again before the previous one finishes.
    _test_future: Option<SpawnedFutureHandle>,
}

impl AiProviderPageView {
    pub fn new(ctx: &mut ViewContext<AiProviderPageView>) -> Self {
        let widget = AiProviderConfigWidget::new(ctx);
        let endpoint_editor = widget.endpoint_editor.clone();
        let api_key_editor = widget.api_key_editor.clone();
        let model_dropdown = widget.model_dropdown.clone();
        AiProviderPageView {
            page: PageType::new_monolith(widget, None, false),
            endpoint_editor,
            api_key_editor,
            model_dropdown,
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
            AiProviderPageAction::SelectModel(model) => {
                let model = model.clone();
                AiProviderSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings.model.set_value(model, ctx);
                });
                push_runtime_config_from_settings(ctx);
            }
            AiProviderPageAction::Connect => {
                log::info!("[ai_provider_page] Connect clicked. Fetching /v1/models.");
                let endpoint = self.endpoint_editor.as_ref(ctx).buffer_text(ctx);
                let api_key = self.api_key_editor.as_ref(ctx).buffer_text(ctx);

                if api_key.trim().is_empty() {
                    self.test_status =
                        TestStatus::Failure("API key is required".to_string());
                    ctx.notify();
                    return;
                }

                self.test_status = TestStatus::InProgress;
                ctx.notify();

                self._test_future = None;

                let saved_model =
                    AiProviderSettings::as_ref(ctx).model.value().clone();
                let model_dropdown_handle = self.model_dropdown.clone();

                use warpui::r#async::FutureExt as _;
                let handle = ctx.spawn(
                    async move {
                        ai_provider::fetch_available_models(&endpoint, &api_key)
                            .with_timeout(std::time::Duration::from_secs(15))
                            .await
                    },
                    move |me, result, ctx| {
                        match result {
                            Ok(Ok(models)) => {
                                let count = models.len();
                                let items = models
                                    .iter()
                                    .map(|m| {
                                        DropdownItem::new(
                                            m.clone(),
                                            AiProviderPageAction::SelectModel(m.clone()),
                                        )
                                    })
                                    .collect::<Vec<_>>();
                                model_dropdown_handle.update(ctx, |dropdown, ctx| {
                                    dropdown.set_items(items, ctx);
                                    if !saved_model.is_empty() {
                                        dropdown.set_selected_by_name(saved_model.clone(), ctx);
                                    } else if let Some(first) = models.first() {
                                        // Auto-select the first model if nothing was saved.
                                        dropdown.set_selected_by_name(first.clone(), ctx);
                                    }
                                });
                                if saved_model.is_empty() {
                                    if let Some(first) = models.first() {
                                        let first = first.clone();
                                        AiProviderSettings::handle(ctx).update(
                                            ctx,
                                            |s, ctx| {
                                                let _ = s.model.set_value(first, ctx);
                                            },
                                        );
                                        push_runtime_config_from_settings(ctx);
                                    }
                                }
                                me.test_status = TestStatus::Success(count);
                            }
                            Ok(Err(e)) => {
                                me.test_status = TestStatus::Failure(format!("{e:#}"));
                            }
                            Err(_timeout) => {
                                me.test_status =
                                    TestStatus::Failure("Connection timed out".to_string());
                            }
                        }
                        ctx.notify();
                    },
                );
                self._test_future = Some(handle);
            }
        }
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
