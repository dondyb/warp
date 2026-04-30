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

// ── Test-connection status (local state only for now) ─────────────────────────

#[derive(Clone, Default)]
#[allow(dead_code)]
enum TestStatus {
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
    #[allow(dead_code)]
    test_status: TestStatus,
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

        let api_key_editor = ctx.add_typed_action_view(move |ctx| {
            let mut editor = EditorView::single_line(make_editor_options(false), ctx);
            editor.set_placeholder_text("sk-...", ctx);
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
            }
        });

        // Save model on every edit.
        ctx.subscribe_to_view(&model_editor, |_me, editor_handle, event, ctx| {
            if let EditorEvent::Edited(_) = event {
                let text = editor_handle.as_ref(ctx).buffer_text(ctx);
                AiProviderSettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings.model.set_value(text, ctx);
                });
            }
        });

        let test_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Test connection", SecondaryTheme)
                .with_size(ButtonSize::Default)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AiProviderPageAction::TestConnection);
                })
        });

        Self {
            endpoint_editor,
            api_key_editor,
            model_editor,
            protocol_dropdown,
            test_button,
            test_status: TestStatus::Idle,
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
        _view: &AiProviderPageView,
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
                .with_style(editor_style.clone())
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
            .with_child(ChildView::new(&self.test_button).finish())
            .finish();

        Container::new(column)
            .with_margin_bottom(HEADER_PADDING)
            .finish()
    }
}

// ── Page view ─────────────────────────────────────────────────────────────────

pub struct AiProviderPageView {
    page: PageType<Self>,
}

impl AiProviderPageView {
    pub fn new(ctx: &mut ViewContext<AiProviderPageView>) -> Self {
        AiProviderPageView {
            page: PageType::new_monolith(AiProviderConfigWidget::new(ctx), None, false),
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
            }
            AiProviderPageAction::TestConnection => {
                // TODO(Task 9): wire up the connection test.
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
