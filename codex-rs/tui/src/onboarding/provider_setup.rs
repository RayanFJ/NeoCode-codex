//! Provider setup widget for custom API configuration.
//!
//! This module provides a simple onboarding flow that asks the user for:
//! 1. Provider API URL (e.g., https://api.example.com/v1)
//! 2. API key
//! Then fetches available models and lets the user pick one.

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;

use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;
use reqwest::Client;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use crate::onboarding::onboarding_screen::KeyboardHandler;
use crate::onboarding::onboarding_screen::StepState;
use crate::onboarding::onboarding_screen::StepStateProvider;
use crate::tui::FrameRequester;

/// A model returned from the provider's /models endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiModel {
    pub id: String,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub owned_by: Option<String>,
}

/// Response format for /models endpoint (OpenAI-compatible).
#[derive(Debug, Clone, Deserialize)]
pub struct ModelsResponse {
    #[serde(alias = "models")]
    pub data: Vec<ApiModel>,
}

/// Current state of the provider setup flow.
#[derive(Clone, Debug)]
pub enum ProviderSetupState {
    /// Entering the API URL.
    EnteringUrl(TextInputState),
    /// Entering the API key.
    EnteringKey(TextInputState),
    /// Fetching models from the API.
    FetchingModels,
    /// Selecting a model from the list.
    SelectingModel(ModelSelectionState),
    /// Setup is complete.
    Complete(ProviderConfig),
}

impl Default for ProviderSetupState {
    fn default() -> Self {
        Self::EnteringUrl(TextInputState::default())
    }
}

/// Text input state for URL or API key entry.
#[derive(Clone, Debug, Default)]
pub struct TextInputState {
    pub value: String,
    pub cursor_position: usize,
}

/// State for model selection.
#[derive(Clone, Debug)]
pub struct ModelSelectionState {
    pub models: Vec<ApiModel>,
    pub selected_index: usize,
}

/// Final provider configuration.
#[derive(Clone, Debug)]
pub struct ProviderConfig {
    pub api_url: String,
    pub api_key: String,
    pub model: String,
}

/// Widget for setting up a custom provider.
pub struct ProviderSetupWidget {
    pub request_frame: FrameRequester,
    pub state: Arc<RwLock<ProviderSetupState>>,
    pub error: Arc<RwLock<Option<String>>>,
    pub codex_home: PathBuf,
    pub animations_enabled: bool,
    // Temporary storage during setup
    api_url: Arc<RwLock<String>>,
    api_key: Arc<RwLock<String>>,
}

impl ProviderSetupWidget {
    pub fn new(
        request_frame: FrameRequester,
        codex_home: PathBuf,
        animations_enabled: bool,
    ) -> Self {
        Self {
            request_frame,
            state: Arc::new(RwLock::new(ProviderSetupState::default())),
            error: Arc::new(RwLock::new(None)),
            codex_home,
            animations_enabled,
            api_url: Arc::new(RwLock::new(String::new())),
            api_key: Arc::new(RwLock::new(String::new())),
        }
    }

    fn set_error(&self, message: Option<String>) {
        *self.error.write().unwrap() = message;
    }

    fn error_message(&self) -> Option<String> {
        self.error.read().unwrap().clone()
    }

    fn submit_url(&mut self) {
        let state = self.state.read().unwrap().clone();
        if let ProviderSetupState::EnteringUrl(input) = state {
            let url = input.value.trim().to_string();
            if url.is_empty() {
                self.set_error(Some("API URL cannot be empty".to_string()));
                self.request_frame.schedule_frame();
                return;
            }
            // Basic URL validation
            if !url.starts_with("http://") && !url.starts_with("https://") {
                self.set_error(Some("URL must start with http:// or https://".to_string()));
                self.request_frame.schedule_frame();
                return;
            }
            *self.api_url.write().unwrap() = url;
            self.set_error(None);
            *self.state.write().unwrap() =
                ProviderSetupState::EnteringKey(TextInputState::default());
            self.request_frame.schedule_frame();
        }
    }

    fn submit_key(&mut self) {
        let state = self.state.read().unwrap().clone();
        if let ProviderSetupState::EnteringKey(input) = state {
            let key = input.value.trim().to_string();
            if key.is_empty() {
                self.set_error(Some("API key cannot be empty".to_string()));
                self.request_frame.schedule_frame();
                return;
            }
            *self.api_key.write().unwrap() = key;
            self.set_error(None);
            self.fetch_models();
        }
    }

    fn fetch_models(&mut self) {
        *self.state.write().unwrap() = ProviderSetupState::FetchingModels;
        self.request_frame.schedule_frame();

        let api_url = self.api_url.read().unwrap().clone();
        let api_key = self.api_key.read().unwrap().clone();
        let state = self.state.clone();
        let error = self.error.clone();
        let request_frame = self.request_frame.clone();

        tokio::spawn(async move {
            let result = fetch_models_from_api(&api_url, &api_key).await;
            match result {
                Ok(models) => {
                    if models.is_empty() {
                        *error.write().unwrap() =
                            Some("No models found at this endpoint".to_string());
                        *state.write().unwrap() =
                            ProviderSetupState::EnteringUrl(TextInputState::default());
                    } else {
                        *error.write().unwrap() = None;
                        *state.write().unwrap() =
                            ProviderSetupState::SelectingModel(ModelSelectionState {
                                models,
                                selected_index: 0,
                            });
                    }
                }
                Err(err) => {
                    *error.write().unwrap() = Some(format!("Failed to fetch models: {err}"));
                    // Go back to URL entry so user can correct
                    *state.write().unwrap() =
                        ProviderSetupState::EnteringUrl(TextInputState::default());
                }
            }
            request_frame.schedule_frame();
        });
    }

    fn select_model(&mut self) {
        let state = self.state.read().unwrap().clone();
        if let ProviderSetupState::SelectingModel(selection) = state {
            if let Some(model) = selection.models.get(selection.selected_index) {
                let api_url = self.api_url.read().unwrap().clone();
                let api_key = self.api_key.read().unwrap().clone();
                let config = ProviderConfig {
                    api_url,
                    api_key,
                    model: model.id.clone(),
                };
                self.save_config(&config);
                *self.state.write().unwrap() = ProviderSetupState::Complete(config);
                self.request_frame.schedule_frame();
            }
        }
    }

    fn save_config(&self, config: &ProviderConfig) {
        let config_path = self.codex_home.join("config.toml");

        // Create config content
        let config_content = format!(
            r#"# Codex configuration - auto-generated
model = "{model}"
approval_policy = "on-failure"
sandbox_mode = "danger-full-access"

model_provider = "custom"

[model_providers.custom]
name = "Custom Provider"
base_url = "{api_url}"
env_key = "CUSTOM_API_KEY"
wire_api = "responses"
"#,
            model = config.model,
            api_url = config.api_url,
        );

        // Also save the API key to a separate file for security
        let auth_path = self.codex_home.join("auth.json");
        let auth_content = format!(
            r#"{{"auth_mode":"apikey","openai_api_key":"{api_key}"}}"#,
            api_key = config.api_key,
        );

        // Ensure codex home directory exists
        if let Err(e) = std::fs::create_dir_all(&self.codex_home) {
            tracing::error!("Failed to create codex home directory: {e}");
            return;
        }

        // Write config
        if let Err(e) = std::fs::write(&config_path, config_content) {
            tracing::error!("Failed to write config.toml: {e}");
        }

        // Write auth
        if let Err(e) = std::fs::write(&auth_path, auth_content) {
            tracing::error!("Failed to write auth.json: {e}");
        }

        // Also set the environment variable for the current session
        // SAFETY: We are the only code setting this env var at startup, no concurrent access
        unsafe {
            std::env::set_var("CUSTOM_API_KEY", &config.api_key);
        }

        tracing::info!("Provider configuration saved to {:?}", config_path);
    }

    fn go_back(&mut self) {
        let state = self.state.read().unwrap().clone();
        match state {
            ProviderSetupState::EnteringKey(_) => {
                let url = self.api_url.read().unwrap().clone();
                *self.state.write().unwrap() = ProviderSetupState::EnteringUrl(TextInputState {
                    value: url,
                    cursor_position: 0,
                });
            }
            ProviderSetupState::SelectingModel(_) => {
                let key = self.api_key.read().unwrap().clone();
                *self.state.write().unwrap() = ProviderSetupState::EnteringKey(TextInputState {
                    value: key,
                    cursor_position: 0,
                });
            }
            _ => {}
        }
        self.set_error(None);
        self.request_frame.schedule_frame();
    }

    fn handle_text_input(&mut self, key_event: &KeyEvent) -> bool {
        let mut state = self.state.write().unwrap();
        let input = match &mut *state {
            ProviderSetupState::EnteringUrl(input) | ProviderSetupState::EnteringKey(input) => {
                input
            }
            _ => return false,
        };

        match key_event.code {
            KeyCode::Char(c)
                if key_event.kind == KeyEventKind::Press
                    && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                    && !key_event.modifiers.contains(KeyModifiers::ALT) =>
            {
                input.value.insert(input.cursor_position, c);
                input.cursor_position += 1;
                drop(state);
                self.set_error(None);
                self.request_frame.schedule_frame();
                true
            }
            KeyCode::Backspace => {
                if input.cursor_position > 0 {
                    input.cursor_position -= 1;
                    input.value.remove(input.cursor_position);
                }
                drop(state);
                self.set_error(None);
                self.request_frame.schedule_frame();
                true
            }
            KeyCode::Delete => {
                if input.cursor_position < input.value.len() {
                    input.value.remove(input.cursor_position);
                }
                drop(state);
                self.set_error(None);
                self.request_frame.schedule_frame();
                true
            }
            KeyCode::Left => {
                if input.cursor_position > 0 {
                    input.cursor_position -= 1;
                }
                drop(state);
                self.request_frame.schedule_frame();
                true
            }
            KeyCode::Right => {
                if input.cursor_position < input.value.len() {
                    input.cursor_position += 1;
                }
                drop(state);
                self.request_frame.schedule_frame();
                true
            }
            KeyCode::Home => {
                input.cursor_position = 0;
                drop(state);
                self.request_frame.schedule_frame();
                true
            }
            KeyCode::End => {
                input.cursor_position = input.value.len();
                drop(state);
                self.request_frame.schedule_frame();
                true
            }
            _ => false,
        }
    }

    fn handle_model_selection(&mut self, key_event: &KeyEvent) -> bool {
        let mut state = self.state.write().unwrap();
        let selection = match &mut *state {
            ProviderSetupState::SelectingModel(selection) => selection,
            _ => return false,
        };

        match key_event.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if selection.selected_index > 0 {
                    selection.selected_index -= 1;
                }
                drop(state);
                self.request_frame.schedule_frame();
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if selection.selected_index < selection.models.len().saturating_sub(1) {
                    selection.selected_index += 1;
                }
                drop(state);
                self.request_frame.schedule_frame();
                true
            }
            KeyCode::PageUp => {
                selection.selected_index = selection.selected_index.saturating_sub(10);
                drop(state);
                self.request_frame.schedule_frame();
                true
            }
            KeyCode::PageDown => {
                selection.selected_index =
                    (selection.selected_index + 10).min(selection.models.len().saturating_sub(1));
                drop(state);
                self.request_frame.schedule_frame();
                true
            }
            _ => false,
        }
    }
}

impl KeyboardHandler for ProviderSetupWidget {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        let state = self.state.read().unwrap().clone();

        match state {
            ProviderSetupState::EnteringUrl(_) => {
                if key_event.code == KeyCode::Enter {
                    self.submit_url();
                } else if key_event.code == KeyCode::Esc {
                    // Could quit here if needed
                } else {
                    self.handle_text_input(&key_event);
                }
            }
            ProviderSetupState::EnteringKey(_) => {
                if key_event.code == KeyCode::Enter {
                    self.submit_key();
                } else if key_event.code == KeyCode::Esc {
                    self.go_back();
                } else {
                    self.handle_text_input(&key_event);
                }
            }
            ProviderSetupState::FetchingModels => {
                // Ignore input while fetching
            }
            ProviderSetupState::SelectingModel(_) => {
                if key_event.code == KeyCode::Enter {
                    self.select_model();
                } else if key_event.code == KeyCode::Esc {
                    self.go_back();
                } else {
                    self.handle_model_selection(&key_event);
                }
            }
            ProviderSetupState::Complete(_) => {
                // Setup is done
            }
        }
    }

    fn handle_paste(&mut self, pasted: String) {
        let trimmed = pasted.trim();
        if trimmed.is_empty() {
            return;
        }

        let mut state = self.state.write().unwrap();
        let input = match &mut *state {
            ProviderSetupState::EnteringUrl(input) | ProviderSetupState::EnteringKey(input) => {
                input
            }
            _ => return,
        };

        input.value.insert_str(input.cursor_position, trimmed);
        input.cursor_position += trimmed.len();
        drop(state);
        self.set_error(None);
        self.request_frame.schedule_frame();
    }
}

impl StepStateProvider for ProviderSetupWidget {
    fn get_step_state(&self) -> StepState {
        let state = self.state.read().unwrap();
        match &*state {
            ProviderSetupState::Complete(_) => StepState::Complete,
            _ => StepState::InProgress,
        }
    }
}

impl WidgetRef for &ProviderSetupWidget {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        let state = self.state.read().unwrap().clone();

        match state {
            ProviderSetupState::EnteringUrl(input) => {
                render_url_input(area, buf, &input, self.error_message());
            }
            ProviderSetupState::EnteringKey(input) => {
                render_key_input(area, buf, &input, self.error_message());
            }
            ProviderSetupState::FetchingModels => {
                render_fetching(area, buf);
            }
            ProviderSetupState::SelectingModel(selection) => {
                render_model_selection(area, buf, &selection, self.error_message());
            }
            ProviderSetupState::Complete(config) => {
                render_complete(area, buf, &config);
            }
        }
    }
}

fn render_url_input(area: Rect, buf: &mut Buffer, input: &TextInputState, error: Option<String>) {
    let [intro_area, input_area, footer_area] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Min(3),
    ])
    .areas(area);

    let intro_lines: Vec<Line> = vec![
        Line::from(vec!["> ".into(), "Enter your Provider API URL".bold()]),
        "".into(),
        "  Example: https://api.example.com/v1".dim().into(),
        "".into(),
    ];
    Paragraph::new(intro_lines)
        .wrap(Wrap { trim: false })
        .render(intro_area, buf);

    let content_line: Line = if input.value.is_empty() {
        vec!["https://...".dim()].into()
    } else {
        Line::from(input.value.clone())
    };
    Paragraph::new(content_line)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title("API URL")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .render(input_area, buf);

    let mut footer_lines: Vec<Line> = vec!["  Press Enter to continue".dim().into()];
    if let Some(err) = error {
        footer_lines.push("".into());
        footer_lines.push(err.red().into());
    }
    Paragraph::new(footer_lines)
        .wrap(Wrap { trim: false })
        .render(footer_area, buf);
}

fn render_key_input(area: Rect, buf: &mut Buffer, input: &TextInputState, error: Option<String>) {
    let [intro_area, input_area, footer_area] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Min(3),
    ])
    .areas(area);

    let intro_lines: Vec<Line> = vec![
        Line::from(vec!["> ".into(), "Enter your API Key".bold()]),
        "".into(),
        "  Your API key will be stored locally.".dim().into(),
        "".into(),
    ];
    Paragraph::new(intro_lines)
        .wrap(Wrap { trim: false })
        .render(intro_area, buf);

    // Mask the API key for display
    let display_value = if input.value.is_empty() {
        "Paste or type your API key".dim().to_string()
    } else {
        "*".repeat(input.value.len())
    };
    Paragraph::new(display_value)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title("API Key")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .render(input_area, buf);

    let mut footer_lines: Vec<Line> = vec![
        "  Press Enter to continue".dim().into(),
        "  Press Esc to go back".dim().into(),
    ];
    if let Some(err) = error {
        footer_lines.push("".into());
        footer_lines.push(err.red().into());
    }
    Paragraph::new(footer_lines)
        .wrap(Wrap { trim: false })
        .render(footer_area, buf);
}

fn render_fetching(area: Rect, buf: &mut Buffer) {
    let lines: Vec<Line> = vec![
        "".into(),
        "  Fetching available models...".cyan().into(),
        "".into(),
    ];
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .render(area, buf);
}

fn render_model_selection(
    area: Rect,
    buf: &mut Buffer,
    selection: &ModelSelectionState,
    error: Option<String>,
) {
    let [intro_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(10),
        Constraint::Length(4),
    ])
    .areas(area);

    let intro_lines: Vec<Line> = vec![
        Line::from(vec!["> ".into(), "Select a model".bold()]),
        "".into(),
    ];
    Paragraph::new(intro_lines)
        .wrap(Wrap { trim: false })
        .render(intro_area, buf);

    // Render model list
    let items: Vec<ListItem> = selection
        .models
        .iter()
        .enumerate()
        .map(|(i, model)| {
            let style = if i == selection.selected_index {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if i == selection.selected_index {
                "> "
            } else {
                "  "
            };
            ListItem::new(format!("{prefix}{}", model.id)).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(format!("Models ({} available)", selection.models.len()))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan)),
    );

    // We need to manage the list state for scrolling
    let mut list_state = ListState::default();
    list_state.select(Some(selection.selected_index));
    ratatui::widgets::StatefulWidget::render(list, list_area, buf, &mut list_state);

    let mut footer_lines: Vec<Line> = vec![
        "  Press Enter to select".dim().into(),
        "  Press Esc to go back".dim().into(),
        "  Use ↑/↓ or j/k to navigate".dim().into(),
    ];
    if let Some(err) = error {
        footer_lines.push("".into());
        footer_lines.push(err.red().into());
    }
    Paragraph::new(footer_lines)
        .wrap(Wrap { trim: false })
        .render(footer_area, buf);
}

fn render_complete(area: Rect, buf: &mut Buffer, config: &ProviderConfig) {
    let lines: Vec<Line> = vec![
        "✓ Provider configured".fg(Color::Green).into(),
        "".into(),
        format!("  API URL: {}", config.api_url).into(),
        format!("  Model: {}", config.model).into(),
        "".into(),
        "  Configuration saved to config.toml".dim().into(),
        "".into(),
        "  Press Enter to continue".cyan().into(),
    ];

    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .render(area, buf);
}

/// Fetch models from the provider API.
async fn fetch_models_from_api(api_url: &str, api_key: &str) -> Result<Vec<ApiModel>, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    // Construct the models endpoint URL
    let url = if api_url.ends_with('/') {
        format!("{api_url}models")
    } else {
        format!("{api_url}/models")
    };

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("API returned error {status}: {body}"));
    }

    let models_response: ModelsResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    Ok(models_response.data)
}

/// Get the provider configuration if setup is complete.
#[allow(dead_code)]
pub fn get_provider_config(widget: &ProviderSetupWidget) -> Option<ProviderConfig> {
    let state = widget.state.read().unwrap();
    if let ProviderSetupState::Complete(config) = &*state {
        Some(config.clone())
    } else {
        None
    }
}
