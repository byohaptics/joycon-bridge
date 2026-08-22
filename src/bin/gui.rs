#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fmt;
use std::fs::{self, File};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use iced::theme::Palette;
use iced::widget::{button, column, container, horizontal_space, radio, row, scrollable, text};
use iced::{Alignment, Border, Color, Element, Font, Length, Task, Theme, border};

const ACTION_BUTTON_WIDTH: f32 = 150.0;
const ACTION_BUTTON_HEIGHT: f32 = 36.0;
const TOGGLE_BUTTON_WIDTH: f32 = 84.0;
const TOGGLE_BUTTON_HEIGHT: f32 = 30.0;
const ACTION_COLOR: Color = Color::from_rgb(0.604, 0.302, 0.0);
const ACTION_HOVER_COLOR: Color = Color::from_rgb(0.455, 0.227, 0.0);
const DANGER_COLOR: Color = Color::from_rgb(0.702, 0.149, 0.118);
const DANGER_HOVER_COLOR: Color = Color::from_rgb(0.510, 0.086, 0.067);
const DISABLED_COLOR: Color = Color::from_rgb(0.357, 0.396, 0.451);
const CONNECTED_COLOR: Color = Color::from_rgb(0.0, 0.420, 0.369);
const PENDING_COLOR: Color = ACTION_COLOR;
const MUTED_COLOR: Color = DISABLED_COLOR;
const CARD_COLOR: Color = Color::WHITE;
const CARD_BORDER_COLOR: Color = Color::from_rgb(0.847, 0.863, 0.890);
const DEFAULT_LISTEN: &str = "0.0.0.0:9010";
const LOG_FILE: &str = "bridge.log";
const MEASUREMENT_FILE: &str = "measurement.csv";
const PROFILE_FILE: &str = "optimized-profile.toml";
const IDLE_STATUS: &str = "Joy-Conの接続を確認してから、振動を開始してください。";
const WAITING_STATUS: &str = "BYO Hapticsからの接続を待っています。";

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args_os().nth(1).is_some() {
        joycon_rumble_bridge::run()?;
        return Ok(());
    }
    let icon = iced::window::icon::from_rgba(
        include_bytes!("../../assets/BYOHAPTICS.rgba").to_vec(),
        64,
        64,
    )?;
    iced::application("Joy-Con Bridge - BYO Haptics", update, view)
        .theme(|_| universal_design_theme())
        .subscription(|_| iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick))
        .default_font(Font::with_name("Yu Gothic UI"))
        .window(iced::window::Settings {
            size: iced::Size::new(720.0, 720.0),
            icon: Some(icon),
            ..iced::window::Settings::default()
        })
        .run_with(|| (App::default(), Task::none()))?;
    Ok(())
}

fn universal_design_theme() -> Theme {
    Theme::custom(
        "Universal Design".into(),
        Palette {
            background: Color::from_rgb8(0xF3, 0xF4, 0xF6),
            text: Color::from_rgb8(0x11, 0x18, 0x27),
            primary: Color::from_rgb8(0x9A, 0x4D, 0x00),
            success: Color::from_rgb8(0x00, 0x6B, 0x5E),
            danger: Color::from_rgb8(0xB3, 0x26, 0x1E),
        },
    )
}

fn action_button_style(
    _theme: &Theme,
    status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    solid_button_style(status, ACTION_COLOR, ACTION_HOVER_COLOR)
}

fn danger_button_style(
    _theme: &Theme,
    status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    solid_button_style(status, DANGER_COLOR, DANGER_HOVER_COLOR)
}

fn solid_button_style(
    status: iced::widget::button::Status,
    active: Color,
    hover: Color,
) -> iced::widget::button::Style {
    let background = match status {
        iced::widget::button::Status::Active => active,
        iced::widget::button::Status::Hovered => hover,
        iced::widget::button::Status::Pressed => hover,
        iced::widget::button::Status::Disabled => DISABLED_COLOR,
    };
    iced::widget::button::Style {
        background: Some(background.into()),
        text_color: Color::WHITE,
        border: border::rounded(4),
        ..iced::widget::button::Style::default()
    }
}

fn secondary_button_style(
    _theme: &Theme,
    status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    let (text_color, border_color) = match status {
        iced::widget::button::Status::Disabled => (DISABLED_COLOR, CARD_BORDER_COLOR),
        iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed => {
            (ACTION_HOVER_COLOR, ACTION_HOVER_COLOR)
        }
        iced::widget::button::Status::Active => (ACTION_COLOR, CARD_BORDER_COLOR),
    };
    iced::widget::button::Style {
        background: None,
        text_color,
        border: Border {
            color: border_color,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..iced::widget::button::Style::default()
    }
}

fn card_style(_theme: &Theme) -> iced::widget::container::Style {
    iced::widget::container::Style {
        background: Some(CARD_COLOR.into()),
        border: Border {
            color: CARD_BORDER_COLOR,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..iced::widget::container::Style::default()
    }
}

/// Every state is encoded by shape and color together, so the display stays
/// readable without color vision. Red is reserved for genuine faults; an idle
/// or waiting state must never look like an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Indicator {
    Active,
    Pending,
    Inactive,
    Fault,
}

impl Indicator {
    fn glyph(self) -> &'static str {
        match self {
            Self::Active => "●",
            Self::Pending => "○",
            Self::Inactive => "－",
            Self::Fault => "×",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Active => CONNECTED_COLOR,
            Self::Pending => PENDING_COLOR,
            Self::Inactive => MUTED_COLOR,
            Self::Fault => DANGER_COLOR,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Side {
    Left,
    #[default]
    Right,
}

impl Side {
    fn argument(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Left => "Joy-Con (L)",
            Self::Right => "Joy-Con (R)",
        })
    }
}

fn data_directory() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("BYO Haptics")
        .join("Joy-Con Bridge")
}

fn data_path(name: &str) -> PathBuf {
    data_directory().join(name)
}

struct App {
    side: Side,
    busy: bool,
    scan_completed: bool,
    left_detected: bool,
    right_detected: bool,
    bridge: Option<Child>,
    bridge_left_connected: bool,
    bridge_right_connected: bool,
    plugin_connected: bool,
    calibration_expanded: bool,
    log_expanded: bool,
    bridge_failed: bool,
    status: String,
    log: String,
}

impl Default for App {
    fn default() -> Self {
        Self {
            side: Side::Right,
            busy: false,
            scan_completed: false,
            left_detected: false,
            right_detected: false,
            bridge: None,
            bridge_left_connected: false,
            bridge_right_connected: false,
            plugin_connected: false,
            calibration_expanded: false,
            log_expanded: false,
            bridge_failed: false,
            status: IDLE_STATUS.into(),
            log: String::new(),
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(child) = self.bridge.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    SideSelected(Side),
    Scan,
    Measure,
    CommandFinished(CommandResult),
    StartBridge,
    StopBridge,
    ToggleCalibration,
    ToggleLog,
    Tick,
}

#[derive(Debug, Clone)]
struct CommandResult {
    title: &'static str,
    success: bool,
    output: String,
}

fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::SideSelected(side) => app.side = side,
        Message::Scan if !app.busy => {
            app.busy = true;
            app.status = "Joy-Conを検索しています…".into();
            return Task::perform(
                run_cli("デバイス検索", vec!["joycon-list".into()]),
                Message::CommandFinished,
            );
        }
        Message::Measure if !app.busy && app.bridge.is_none() => {
            if let Err(error) = fs::create_dir_all(data_directory()) {
                app.status = "測定を開始できませんでした。".into();
                app.log = error.to_string();
                app.log_expanded = true;
                return Task::none();
            }
            app.busy = true;
            app.status = format!(
                "{}を測定しています。約1分間、動かさないでください…",
                app.side
            );
            let args = vec![
                "joycon-imu-sweep".into(),
                "--side".into(),
                app.side.argument().into(),
                "--output".into(),
                data_path(MEASUREMENT_FILE).to_string_lossy().into_owned(),
                "--profile".into(),
                data_path(PROFILE_FILE).to_string_lossy().into_owned(),
            ];
            return Task::perform(run_cli("振動の最適化", args), Message::CommandFinished);
        }
        Message::CommandFinished(result) => {
            app.busy = false;
            if !result.success {
                app.log_expanded = true;
            }
            if result.title == "デバイス検索" {
                app.scan_completed = true;
                app.left_detected = result.output.contains("Left Joy-Con");
                app.right_detected = result.output.contains("Right Joy-Con");
                if app.side == Side::Left && !app.left_detected && app.right_detected {
                    app.side = Side::Right;
                } else if app.side == Side::Right && !app.right_detected && app.left_detected {
                    app.side = Side::Left;
                }
            }
            app.status = if result.title == "デバイス検索" {
                if !result.success {
                    "Joy-Conの確認に失敗しました。下の動作ログを確認してください。".into()
                } else if app.left_detected || app.right_detected {
                    "接続済みJoy-Conを更新しました。".into()
                } else {
                    "Joy-Conが見つかりません。ボタンを押して再検索してください。".into()
                }
            } else if result.success {
                format!("{}が正常に完了しました。", result.title)
            } else {
                format!(
                    "{}に失敗しました。下の動作ログを確認してください。",
                    result.title
                )
            };
            app.log = match (result.title, result.success) {
                ("デバイス検索", true) if app.left_detected && app.right_detected => {
                    "左右両方のJoy-Conが見つかりました。".into()
                }
                ("デバイス検索", true) if app.left_detected => {
                    "Joy-Con (L)だけが見つかりました。Joy-Con (R)を接続してください。".into()
                }
                ("デバイス検索", true) if app.right_detected => {
                    "Joy-Con (R)だけが見つかりました。Joy-Con (L)を接続してください。".into()
                }
                ("デバイス検索", true) => "Joy-Conが見つかりませんでした。".into(),
                ("振動の最適化", true) => "振動の最適化が完了しました。".into(),
                _ => localize_cli_output(&result.output),
            };
        }
        Message::StartBridge if !app.busy && app.bridge.is_none() => {
            app.bridge_failed = false;
            app.scan_completed = false;
            app.left_detected = false;
            app.right_detected = false;
            match start_bridge() {
                Ok(child) => {
                    app.bridge = Some(child);
                    app.bridge_left_connected = false;
                    app.bridge_right_connected = false;
                    app.plugin_connected = false;
                    app.status = WAITING_STATUS.into();
                }
                Err(error) => {
                    app.status = "ブリッジを起動できませんでした。".into();
                    app.log = error.to_string();
                    app.log_expanded = true;
                }
            }
        }
        Message::StopBridge => {
            if let Some(mut child) = app.bridge.take() {
                let _ = child.kill();
                let _ = child.wait();
                app.status = IDLE_STATUS.into();
            }
        }
        Message::ToggleCalibration => {
            app.calibration_expanded = !app.calibration_expanded;
        }
        Message::ToggleLog => {
            app.log_expanded = !app.log_expanded;
        }
        Message::Tick => {
            if let Some(child) = app.bridge.as_mut()
                && let Ok(Some(_)) = child.try_wait()
            {
                app.bridge = None;
                app.plugin_connected = false;
                app.bridge_failed = true;
                app.log_expanded = true;
                app.status = "振動が予期せず停止しました。動作ログを確認してください。".into();
                app.log = fs::read_to_string(data_path(LOG_FILE))
                    .map(|log| localize_cli_output(&log))
                    .unwrap_or_else(|error| error.to_string());
            }
            if app.bridge.is_some()
                && let Ok(log) = fs::read_to_string(data_path(LOG_FILE))
            {
                let was_connected = app.plugin_connected;
                app.bridge_left_connected = last_status(&log, "joycon-left") == Some("connected");
                app.bridge_right_connected = last_status(&log, "joycon-right") == Some("connected");
                app.plugin_connected = last_status(&log, "plugin") == Some("connected");
                if app.plugin_connected != was_connected {
                    app.status = if app.plugin_connected {
                        "BYO Hapticsからの信号でJoy-Conが振動します。".into()
                    } else {
                        WAITING_STATUS.into()
                    };
                }
            }
        }
        _ => {}
    }
    Task::none()
}

fn connection_state(
    bridge_running: bool,
    bridge_connected: bool,
    scan_completed: bool,
    detected: bool,
) -> (Indicator, &'static str) {
    if bridge_running && bridge_connected {
        (Indicator::Active, "接続")
    } else if bridge_running {
        (Indicator::Pending, "接続待ち")
    } else if !scan_completed {
        (Indicator::Inactive, "未確認")
    } else if detected {
        (Indicator::Active, "検出済み")
    } else {
        (Indicator::Fault, "未検出")
    }
}

/// The headline answers "is it working right now". The line under it is
/// `App::status`, which every transition keeps current, so the two never
/// disagree and nothing has to be said twice.
fn bridge_state(app: &App) -> (Indicator, &'static str) {
    if app.bridge.is_some() {
        if app.plugin_connected {
            (Indicator::Active, "振動中")
        } else {
            (Indicator::Pending, "待機中")
        }
    } else if app.bridge_failed {
        (Indicator::Fault, "停止しました")
    } else {
        (Indicator::Inactive, "停止中")
    }
}

fn status_row<'a>(label: &'a str, indicator: Indicator, state: &'a str) -> Element<'a, Message> {
    row![
        text(label).size(15),
        horizontal_space(),
        text(indicator.glyph()).size(15).color(indicator.color()),
        text(state).size(15).color(indicator.color()),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

fn section<'a>(
    title: &'a str,
    expanded: bool,
    toggle: Message,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    let header = row![
        text(title).size(18),
        horizontal_space(),
        button(text(if expanded { "閉じる" } else { "開く" }).size(14))
            .on_press(toggle)
            .width(Length::Fixed(TOGGLE_BUTTON_WIDTH))
            .height(Length::Fixed(TOGGLE_BUTTON_HEIGHT))
            .style(secondary_button_style),
    ]
    .align_y(Alignment::Center);

    let inner = if expanded {
        column![header, body].spacing(16)
    } else {
        column![header]
    };

    container(inner)
        .width(Length::Fill)
        .padding(16)
        .style(card_style)
        .into()
}

fn view(app: &App) -> Element<'_, Message> {
    let bridge_running = app.bridge.is_some();
    let selected_detected = match app.side {
        Side::Left => app.left_detected,
        Side::Right => app.right_detected,
    };

    let (bridge_indicator, bridge_title) = bridge_state(app);
    let bridge_button = (if bridge_running {
        button("振動を停止").on_press(Message::StopBridge)
    } else if app.busy {
        button("振動を開始")
    } else {
        button("振動を開始").on_press(Message::StartBridge)
    })
    .width(Length::Fixed(ACTION_BUTTON_WIDTH))
    .height(Length::Fixed(ACTION_BUTTON_HEIGHT))
    .style(if bridge_running {
        danger_button_style
    } else {
        action_button_style
    });

    let bridge_card = container(
        column![
            row![
                text(bridge_indicator.glyph())
                    .size(26)
                    .color(bridge_indicator.color()),
                text(bridge_title).size(26),
                horizontal_space(),
                bridge_button,
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            text(&app.status).size(14).color(MUTED_COLOR),
        ]
        .spacing(14),
    )
    .width(Length::Fill)
    .padding(24)
    .style(card_style);

    let (left_indicator, left_state) = connection_state(
        bridge_running,
        app.bridge_left_connected,
        app.scan_completed,
        app.left_detected,
    );
    let (right_indicator, right_state) = connection_state(
        bridge_running,
        app.bridge_right_connected,
        app.scan_completed,
        app.right_detected,
    );
    let (plugin_indicator, plugin_state) = if !bridge_running {
        (Indicator::Inactive, "未確認")
    } else if app.plugin_connected {
        (Indicator::Active, "接続")
    } else {
        (Indicator::Pending, "接続待ち")
    };

    let scan = (if app.busy {
        button("接続を確認")
    } else {
        button("接続を確認").on_press(Message::Scan)
    })
    .width(Length::Fixed(ACTION_BUTTON_WIDTH))
    .height(Length::Fixed(ACTION_BUTTON_HEIGHT))
    .style(action_button_style);

    let connection_card = container(
        column![
            status_row("Joy-Con (L)", left_indicator, left_state),
            status_row("Joy-Con (R)", right_indicator, right_state),
            status_row("BYO Haptics", plugin_indicator, plugin_state),
            row![horizontal_space(), scan],
        ]
        .spacing(14),
    )
    .width(Length::Fill)
    .padding(16)
    .style(card_style);

    let measure = (if app.busy || bridge_running || !selected_detected {
        button("測定して最適化")
    } else {
        button("測定して最適化").on_press(Message::Measure)
    })
    .width(Length::Fixed(ACTION_BUTTON_WIDTH))
    .height(Length::Fixed(ACTION_BUTTON_HEIGHT))
    .style(action_button_style);

    let calibration_body = column![
        text("通常は必要ありません。振動の強さを調整したい場合だけ実行してください。")
            .size(14)
            .color(MUTED_COLOR),
        row![
            text("測定対象").size(15).width(Length::Fixed(120.0)),
            radio(
                "Joy-Con (L)",
                Side::Left,
                Some(app.side),
                Message::SideSelected
            ),
            radio(
                "Joy-Con (R)",
                Side::Right,
                Some(app.side),
                Message::SideSelected
            ),
        ]
        .spacing(18)
        .align_y(Alignment::Center),
        text("Joy-Conを安定した場所に置き、測定中は動かさないでください。")
            .size(14)
            .color(MUTED_COLOR),
        row![horizontal_space(), measure],
    ]
    .spacing(14);

    let log_body = scrollable(
        text(if app.log.is_empty() {
            "まだ記録はありません。"
        } else {
            &app.log
        })
        .size(14),
    )
    .height(Length::Fixed(180.0));

    // A label belongs to the card underneath it, so it sits closer to that card
    // than the gap that separates one group from the next.
    let connection_group = column![
        text("接続状況").size(14).color(MUTED_COLOR),
        connection_card,
    ]
    .spacing(6);

    let content = column![
        row![
            text("Joy-Con Bridge").size(24),
            horizontal_space(),
            text(concat!("v", env!("CARGO_PKG_VERSION")))
                .size(14)
                .color(MUTED_COLOR),
        ]
        .align_y(Alignment::Center),
        bridge_card,
        connection_group,
        section(
            "振動の測定と最適化",
            app.calibration_expanded,
            Message::ToggleCalibration,
            calibration_body.into(),
        ),
        section(
            "動作ログ",
            app.log_expanded,
            Message::ToggleLog,
            log_body.into(),
        ),
    ]
    .spacing(18)
    .padding(24)
    .max_width(640);

    scrollable(container(content).center_x(Length::Fill))
        .height(Length::Fill)
        .into()
}

async fn run_cli(title: &'static str, args: Vec<String>) -> CommandResult {
    let result = bridge_command().args(args).output();
    match result {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            CommandResult {
                title,
                success: output.status.success(),
                output: text,
            }
        }
        Err(error) => CommandResult {
            title,
            success: false,
            output: error.to_string(),
        },
    }
}

fn localize_cli_output(output: &str) -> String {
    output
        .replace(
            "No Joy-Con HID devices found via hidapi.",
            "Joy-Conが見つかりませんでした。",
        )
        .replace(
            "both configured Joy-Cons must be connected",
            "設定された左右両方のJoy-Conを接続してください",
        )
        .replace(
            "Expected Nintendo VID 057e with PID 2006 (L) or 2007 (R).",
            "Nintendo VID 057e、PID 2006（L）または2007（R）を検索しました。",
        )
        .replace("Found ", "検出数: ")
        .replace(" Joy-Con HID device(s):", " 台のJoy-Con")
        .replace("Left Joy-Con", "Joy-Con (L)")
        .replace("Right Joy-Con", "Joy-Con (R)")
        .replace("product=", "製品=")
        .replace("serial=", "シリアル=")
        .replace("path=", "パス=")
        .replace("open: ok", "接続可能")
        .replace("open: failed", "接続失敗")
        .replace("Keep the ", "")
        .replace(
            " stationary. Stabilizing IMU...",
            "を動かさないでください。IMUを安定化中…",
        )
        .replace("Measuring three baseline windows...", "基準値を3回測定中…")
        .replace("baseline samples=", "基準サンプル数=")
        .replace(" noise_rms_lsb=", " ノイズRMS=")
        .replace("amplitude=", "振幅=")
        .replace(" low=", " Low=")
        .replace(" high=", " High=")
        .replace(" samples=", " サンプル数=")
        .replace(" rms=", " RMS=")
        .replace("IMU sweep finished:", "IMU測定結果:")
        .replace("optimized profile saved:", "最適化プロファイル保存先:")
}

fn start_bridge() -> std::io::Result<Child> {
    fs::create_dir_all(data_directory())?;
    let log = File::create(data_path(LOG_FILE))?;
    let error_log = log.try_clone()?;
    bridge_command()
        .args(["--listen", DEFAULT_LISTEN, "--imu-profile"])
        .arg(data_path(PROFILE_FILE))
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log))
        .spawn()
}

fn last_status<'a>(log: &'a str, key: &str) -> Option<&'a str> {
    log.lines()
        .rev()
        .filter(|line| line.starts_with("bridge-status "))
        .flat_map(str::split_whitespace)
        .find_map(|field| match field.split_once('=') {
            Some((name, value)) if name == key => Some(value),
            _ => None,
        })
}

fn bridge_command() -> Command {
    let mut command = Command::new(bridge_executable());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn bridge_executable() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("Joy-Con-Bridge.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn localizes_device_and_measurement_output() {
        let localized = localize_cli_output(
            "Found 1 Joy-Con HID device(s):\nLeft Joy-Con open: ok\n\
             amplitude=1.00 low=90.0 high=180.0 samples=100 rms=1200.0\n\
             optimized profile saved: joycon-rumble-profiles.toml",
        );
        assert!(localized.contains("検出数: 1 台のJoy-Con"));
        assert!(localized.contains("Joy-Con (L) 接続可能"));
        assert!(localized.contains("振幅=1.00 Low=90.0 High=180.0 サンプル数=100 RMS=1200.0"));
        assert!(localized.contains("最適化プロファイル保存先:"));
    }

    #[test]
    fn reads_latest_bridge_status_value() {
        let log = "bridge-status plugin=connected\nbridge-status plugin=disconnected\n";
        assert_eq!(last_status(log, "plugin"), Some("disconnected"));
    }

    #[test]
    fn stopped_bridge_reports_detection_snapshot_not_connection() {
        assert_eq!(
            connection_state(false, false, true, true),
            (Indicator::Active, "検出済み")
        );
        assert_eq!(
            connection_state(true, true, false, false),
            (Indicator::Active, "接続")
        );
    }

    #[test]
    fn waiting_for_a_peer_is_not_reported_as_a_fault() {
        // The bridge is up but nothing has connected yet. That is a pending
        // state, not an error, so it must not borrow the fault indicator.
        let (indicator, _) = connection_state(true, false, false, false);
        assert_eq!(indicator, Indicator::Pending);

        let idle = App::default();
        let (indicator, title) = bridge_state(&idle);
        assert_eq!(indicator, Indicator::Inactive);
        assert_eq!(title, "停止中");
    }

    #[test]
    fn every_indicator_has_a_distinct_shape_as_well_as_a_color() {
        use Indicator::{Active, Fault, Inactive, Pending};
        let glyphs = [Active, Pending, Inactive, Fault].map(Indicator::glyph);
        let unique: std::collections::HashSet<_> = glyphs.iter().collect();
        assert_eq!(unique.len(), glyphs.len());
    }

    #[test]
    fn a_failure_reveals_the_log_it_tells_the_user_to_read() {
        let mut app = App::default();
        assert!(!app.log_expanded);
        let _ = update(
            &mut app,
            Message::CommandFinished(CommandResult {
                title: "デバイス検索",
                success: false,
                output: "failed".into(),
            }),
        );
        assert!(app.log_expanded);
    }

    #[test]
    fn bridge_runs_from_the_same_application() {
        assert_eq!(bridge_executable(), std::env::current_exe().unwrap());
    }

    #[test]
    fn calibration_options_are_hidden_by_default() {
        let mut app = App::default();
        assert!(!app.calibration_expanded);
        let _ = update(&mut app, Message::ToggleCalibration);
        assert!(app.calibration_expanded);
    }

    #[test]
    fn indicator_colors_meet_wcag_normal_text_contrast_on_a_card() {
        for indicator in [
            Indicator::Active,
            Indicator::Pending,
            Indicator::Inactive,
            Indicator::Fault,
        ] {
            assert!(contrast_ratio(indicator.color(), CARD_COLOR) >= 4.5);
        }
    }

    #[test]
    fn button_text_colors_meet_wcag_normal_text_contrast() {
        for background in [
            ACTION_COLOR,
            ACTION_HOVER_COLOR,
            DANGER_COLOR,
            DANGER_HOVER_COLOR,
            DISABLED_COLOR,
        ] {
            assert!(contrast_ratio(Color::WHITE, background) >= 4.5);
        }
    }

    fn contrast_ratio(first: Color, second: Color) -> f32 {
        let first = relative_luminance(first);
        let second = relative_luminance(second);
        (first.max(second) + 0.05) / (first.min(second) + 0.05)
    }

    fn relative_luminance(color: Color) -> f32 {
        fn linear(value: f32) -> f32 {
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    }
}
