//! Settings page for configuring a custom AI provider (OpenAI- or
//! Anthropic-compatible endpoint). Replaces the env-var-only path
//! from M1b-chat.
//!
//! Form fields and persistence are added in Task 6+. This file
//! provides the minimal skeleton so that the "AI Provider" sidebar
//! tab opens without panicking.

use super::{
    settings_page::{
        MatchData, PageType, SettingsPageEvent, SettingsPageMeta, SettingsPageViewHandle,
        SettingsWidget,
    },
    SettingsSection,
};
use crate::appearance::Appearance;
use warpui::{
    elements::{
        Align, Element, Flex, MainAxisAlignment, ParentElement,
    },
    ui_components::components::UiComponent,
    AppContext, Entity, View, ViewContext, ViewHandle,
};

pub struct AiProviderPageView {
    page: PageType<Self>,
}

impl AiProviderPageView {
    pub fn new(_ctx: &mut ViewContext<AiProviderPageView>) -> Self {
        AiProviderPageView {
            page: PageType::new_monolith(AiProviderPlaceholderWidget, None, false),
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

struct AiProviderPlaceholderWidget;

impl SettingsWidget for AiProviderPlaceholderWidget {
    type View = AiProviderPageView;

    fn search_terms(&self) -> &str {
        "ai provider custom endpoint openai anthropic"
    }

    fn render(
        &self,
        _view: &AiProviderPageView,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let label = ui_builder
            .span("AI Provider settings - form coming in next task".to_string())
            .with_soft_wrap()
            .build()
            .finish();

        Align::new(
            Flex::column()
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_child(label)
                .finish(),
        )
        .finish()
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
