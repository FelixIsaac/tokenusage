use anyhow::{Result, anyhow};
use iced::alignment::{Horizontal, Vertical};
use iced::widget::{
    button, checkbox, column, container, mouse_area, row, scrollable, text, text_input,
    vertical_space,
};
use iced::{Color, Element, Font, Length, Subscription, Task, Theme, window};

use crate::cli::{CommonArgs, GuiArgs, SortOrder};
use crate::pipeline::{ReportPeriod, collect_report};
use crate::types::{DailyReport, DailyRow, TableLayout, TokenCounts};

#[derive(Debug, Clone)]
enum Message {
    SinceChanged(String),
    UntilChanged(String),
    ProjectChanged(String),
    InstancesToggled(bool),
    WindowResized(f32),
    UseDaily,
    UseMonthly,
    UseWeekly,
    Refresh,
    ChartBarHovered(ChartKind, usize),
    ChartBarHoverCleared(ChartKind),
    Ignore,
    ReportLoaded(Result<DailyReport, String>),
}

#[derive(Debug, Clone)]
struct ReportRequest {
    common: CommonArgs,
    period: ReportPeriod,
    instances: bool,
    project: Option<String>,
    start_of_week: crate::cli::WeekStart,
}

#[derive(Debug)]
struct GuiState {
    common: CommonArgs,
    period: ReportPeriod,
    start_of_week: crate::cli::WeekStart,
    instances: bool,
    since: String,
    until: String,
    project: String,
    loading: bool,
    report: Option<DailyReport>,
    tokens_chart: TrendChart,
    cost_chart: TrendChart,
    hovered_tokens_bar: Option<usize>,
    hovered_cost_bar: Option<usize>,
    window_width: f32,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum TrendMetric {
    Tokens,
    CostUsd,
}

#[derive(Debug, Clone, Copy)]
enum ChartKind {
    Tokens,
    Cost,
}

#[derive(Debug, Clone)]
struct TrendChart {
    title: &'static str,
    metric: TrendMetric,
    labels: Vec<String>,
    values: Vec<f64>,
    color: Color,
}

#[derive(Debug, Clone)]
struct SampledBar {
    label: String,
    value: f64,
}

const CHART_HEIGHT: f32 = 220.0;
const CHART_CARD_HEIGHT: f32 = 270.0;

impl GuiState {
    fn from_args(args: GuiArgs) -> Self {
        let mut common = args.common.clone();
        common.order = SortOrder::Desc;

        Self {
            common,
            period: match args.period {
                crate::cli::GuiPeriod::Daily => ReportPeriod::Daily,
                crate::cli::GuiPeriod::Monthly => ReportPeriod::Monthly,
                crate::cli::GuiPeriod::Weekly => ReportPeriod::Weekly,
            },
            start_of_week: args.start_of_week,
            instances: args.instances,
            since: args.common.since.unwrap_or_default(),
            until: args.common.until.unwrap_or_default(),
            project: args.project.unwrap_or_default(),
            loading: true,
            report: None,
            tokens_chart: TrendChart::empty_tokens(),
            cost_chart: TrendChart::empty_cost(),
            hovered_tokens_bar: None,
            hovered_cost_bar: None,
            window_width: 1600.0,
            error: None,
        }
    }

    fn request(&self) -> ReportRequest {
        let mut common = self.common.clone();
        common.since = normalize_optional_string(&self.since);
        common.until = normalize_optional_string(&self.until);

        ReportRequest {
            common,
            period: self.period,
            instances: self.instances,
            project: normalize_optional_string(&self.project),
            start_of_week: self.start_of_week,
        }
    }
}

pub(crate) fn run_gui(args: GuiArgs) -> Result<()> {
    iced::application("tu gui (Iced/tiny-skia)", update, view)
        .theme(|_| Theme::TokyoNightStorm)
        .subscription(subscription)
        .antialiasing(true)
        .run_with(|| {
            let state = GuiState::from_args(args);
            let task = Task::perform(load_report(state.request()), Message::ReportLoaded);
            (state, task)
        })
        .map_err(|err| anyhow!("failed to run GUI: {err}"))
}

fn subscription(_state: &GuiState) -> Subscription<Message> {
    Subscription::batch([
        window::resize_events().map(|(_id, size)| Message::WindowResized(size.width)),
        window::events().map(|(_id, event)| match event {
            window::Event::Opened { size, .. } => Message::WindowResized(size.width),
            _ => Message::Ignore,
        }),
    ])
}

fn update(state: &mut GuiState, message: Message) -> Task<Message> {
    match message {
        Message::SinceChanged(value) => {
            state.since = value;
            Task::none()
        }
        Message::UntilChanged(value) => {
            state.until = value;
            Task::none()
        }
        Message::ProjectChanged(value) => {
            state.project = value;
            Task::none()
        }
        Message::InstancesToggled(value) => {
            state.instances = value;
            Task::none()
        }
        Message::WindowResized(width) => {
            state.window_width = width.max(320.0);
            Task::none()
        }
        Message::UseDaily => {
            if state.period == ReportPeriod::Daily {
                return Task::none();
            }
            state.period = ReportPeriod::Daily;
            state.hovered_tokens_bar = None;
            state.hovered_cost_bar = None;
            state.loading = true;
            state.error = None;
            Task::perform(load_report(state.request()), Message::ReportLoaded)
        }
        Message::UseMonthly => {
            if state.period == ReportPeriod::Monthly {
                return Task::none();
            }
            state.period = ReportPeriod::Monthly;
            state.hovered_tokens_bar = None;
            state.hovered_cost_bar = None;
            state.loading = true;
            state.error = None;
            Task::perform(load_report(state.request()), Message::ReportLoaded)
        }
        Message::UseWeekly => {
            if state.period == ReportPeriod::Weekly {
                return Task::none();
            }
            state.period = ReportPeriod::Weekly;
            state.hovered_tokens_bar = None;
            state.hovered_cost_bar = None;
            state.loading = true;
            state.error = None;
            Task::perform(load_report(state.request()), Message::ReportLoaded)
        }
        Message::Refresh => {
            state.loading = true;
            state.error = None;
            state.hovered_tokens_bar = None;
            state.hovered_cost_bar = None;
            Task::perform(load_report(state.request()), Message::ReportLoaded)
        }
        Message::ChartBarHovered(kind, index) => {
            match kind {
                ChartKind::Tokens => state.hovered_tokens_bar = Some(index),
                ChartKind::Cost => state.hovered_cost_bar = Some(index),
            }
            Task::none()
        }
        Message::ChartBarHoverCleared(kind) => {
            match kind {
                ChartKind::Tokens => state.hovered_tokens_bar = None,
                ChartKind::Cost => state.hovered_cost_bar = None,
            }
            Task::none()
        }
        Message::Ignore => Task::none(),
        Message::ReportLoaded(result) => {
            state.loading = false;
            match result {
                Ok(report) => {
                    let (tokens_chart, cost_chart) =
                        build_trend_charts(&report, state.common.order);
                    state.report = Some(report);
                    state.tokens_chart = tokens_chart;
                    state.cost_chart = cost_chart;
                    state.hovered_tokens_bar = None;
                    state.hovered_cost_bar = None;
                    state.error = None;
                }
                Err(err) => {
                    state.report = None;
                    state.tokens_chart = TrendChart::empty_tokens();
                    state.cost_chart = TrendChart::empty_cost();
                    state.hovered_tokens_bar = None;
                    state.hovered_cost_bar = None;
                    state.error = Some(err);
                }
            }
            scrollable::snap_to(report_scroll_id(), scrollable::RelativeOffset::START)
        }
    }
}

fn view(state: &GuiState) -> Element<'_, Message> {
    let table_layout = table_layout_for_width(state.window_width);

    let status_line = if state.loading {
        "Loading report...".to_string()
    } else if let Some(report) = &state.report {
        format!(
            "{} rows · {} files",
            report.daily.len(),
            report.stats.files_discovered
        )
    } else {
        "No report loaded".to_string()
    };

    let title = row![
        text("Token Usage").size(30).font(Font::MONOSPACE),
        container(text(status_line).size(14).font(Font::MONOSPACE))
            .padding([6, 10])
            .style(style_status_chip)
    ]
    .spacing(12)
    .align_y(Vertical::Center)
    .width(Length::Fill);

    let period_controls = row![
        period_button(
            "Daily",
            state.period == ReportPeriod::Daily,
            Message::UseDaily
        ),
        period_button(
            "Weekly",
            state.period == ReportPeriod::Weekly,
            Message::UseWeekly
        ),
        period_button(
            "Monthly",
            state.period == ReportPeriod::Monthly,
            Message::UseMonthly,
        ),
        button(text(if state.loading {
            "Loading..."
        } else {
            "Refresh"
        }))
        .style(if state.loading {
            button::secondary
        } else {
            button::success
        })
        .on_press_maybe((!state.loading).then_some(Message::Refresh)),
    ]
    .spacing(8)
    .align_y(Vertical::Center);

    let filters = row![
        text_input("since (YYYY-MM-DD)", &state.since)
            .on_input(Message::SinceChanged)
            .width(Length::FillPortion(2)),
        text_input("until (YYYY-MM-DD)", &state.until)
            .on_input(Message::UntilChanged)
            .width(Length::FillPortion(2)),
        checkbox("instances", state.instances).on_toggle(Message::InstancesToggled),
        text_input("project contains...", &state.project)
            .on_input(Message::ProjectChanged)
            .width(Length::FillPortion(3)),
    ]
    .spacing(10)
    .align_y(Vertical::Center)
    .width(Length::Fill);

    let controls = container(
        column![period_controls, filters]
            .spacing(10)
            .width(Length::Fill),
    )
    .padding(14)
    .width(Length::Fill)
    .style(style_controls_panel);

    let table_header = build_table_header(table_layout);

    let mut body = column![].spacing(6).width(Length::Fill);
    if let Some(report) = &state.report {
        for (idx, row_data) in report.daily.iter().enumerate() {
            body = body.push(
                container(report_row(row_data, table_layout))
                    .padding([8, 10])
                    .width(Length::Fill)
                    .style(move |theme| style_data_row(theme, idx)),
            );
        }
        body = body.push(
            container(report_totals_row(&report.totals, table_layout))
                .padding([10, 10])
                .width(Length::Fill)
                .style(style_total_row),
        );
    } else if state.loading {
        body = body.push(
            container(text("Loading report...").size(18))
                .padding([16, 12])
                .width(Length::Fill)
                .style(style_placeholder),
        );
    } else {
        body = body.push(
            container(text("No report loaded").size(18))
                .padding([16, 12])
                .width(Length::Fill)
                .style(style_placeholder),
        );
    }

    let mut content = column![title, summary_cards(state), controls, charts_panel(state)]
        .spacing(12)
        .width(Length::Fill);

    if let Some(err) = &state.error {
        content = content.push(
            container(text(format!("Error: {err}")).size(14))
                .padding([10, 12])
                .width(Length::Fill)
                .style(style_error_panel),
        );
    }

    content = content.push(table_header);
    content = content.push(
        scrollable(body)
            .id(report_scroll_id())
            .height(Length::Fill)
            .width(Length::Fill),
    );

    container(content)
        .style(style_root)
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

async fn load_report(request: ReportRequest) -> Result<DailyReport, String> {
    collect_report(
        request.common,
        request.period,
        request.instances,
        request.project,
        request.start_of_week,
    )
    .await
    .map_err(|err| err.to_string())
}

fn charts_panel(state: &GuiState) -> Element<'_, Message> {
    row![
        chart_card(
            &state.tokens_chart,
            ChartKind::Tokens,
            state.hovered_tokens_bar,
        ),
        chart_card(&state.cost_chart, ChartKind::Cost, state.hovered_cost_bar),
    ]
    .spacing(10)
    .align_y(Vertical::Top)
    .clip(true)
    .height(Length::Fixed(CHART_CARD_HEIGHT))
    .width(Length::Fill)
    .into()
}

fn chart_card<'a>(
    chart: &'a TrendChart,
    kind: ChartKind,
    hovered: Option<usize>,
) -> Element<'a, Message> {
    let sampled = sample_series_with_labels(&chart.labels, &chart.values, 56);
    let hover_text = hovered
        .and_then(|index| sampled.get(index))
        .map(|bar| {
            format!(
                "{}: {}",
                bar.label,
                format_metric_value(chart.metric, bar.value)
            )
        })
        .unwrap_or_else(|| "hover bar".to_string());

    let body: Element<'a, Message> = if !sampled.is_empty() {
        trend_bars(kind, chart.metric, chart.color, &sampled, hovered)
    } else {
        container(text("No data yet").size(16))
            .height(Length::Fixed(CHART_HEIGHT))
            .center_y(Length::Fill)
            .into()
    };

    container(
        column![
            row![
                text(chart.title).size(14).font(Font::MONOSPACE),
                container(text(hover_text).size(12).font(Font::MONOSPACE))
                    .align_x(Horizontal::Right)
                    .width(Length::Fill),
            ]
            .align_y(Vertical::Center),
            body,
        ]
        .spacing(8)
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true),
    )
    .height(Length::Fixed(CHART_CARD_HEIGHT))
    .padding([10, 12])
    .clip(true)
    .width(Length::FillPortion(1))
    .style(style_chart_panel)
    .into()
}

fn build_trend_charts(report: &DailyReport, order: SortOrder) -> (TrendChart, TrendChart) {
    let mut rows = report.daily.iter().collect::<Vec<_>>();
    if order == SortOrder::Desc {
        rows.reverse();
    }

    let labels = rows
        .iter()
        .map(|row| compact_date_label(&row.date))
        .collect::<Vec<_>>();
    let token_values = rows
        .iter()
        .map(|row| row.totals.total_tokens as f64)
        .collect::<Vec<_>>();
    let cost_values = rows
        .iter()
        .map(|row| row.totals.cost_usd)
        .collect::<Vec<_>>();

    (
        TrendChart::new(
            "Token Trend",
            TrendMetric::Tokens,
            labels.clone(),
            token_values,
            Color::from_rgb8(0x2A, 0xC3, 0xDE),
        ),
        TrendChart::new(
            "Cost Trend (USD)",
            TrendMetric::CostUsd,
            labels,
            cost_values,
            Color::from_rgb8(0x9E, 0xCE, 0x6A),
        ),
    )
}

fn trend_bars(
    kind: ChartKind,
    metric: TrendMetric,
    color: Color,
    sampled: &[SampledBar],
    hovered: Option<usize>,
) -> Element<'static, Message> {
    const PLOT_PAD_V: u16 = 10;
    const PLOT_PAD_H: u16 = 12;

    let max_value = sampled
        .iter()
        .map(|item| item.value)
        .fold(0.0, f64::max)
        .max(1.0);
    let chart_body_height = CHART_HEIGHT - 74.0;

    let mut bars = row![]
        .spacing(2)
        .width(Length::Fill)
        .height(Length::Fixed(chart_body_height));
    for (index, sample) in sampled.iter().enumerate() {
        let is_hovered = hovered == Some(index);
        let bar_color = with_alpha(color, if is_hovered { 1.0 } else { 0.82 });
        let bar_height = ((sample.value / max_value) as f32 * (chart_body_height - 4.0)).max(2.0);
        let bar = column![
            vertical_space().height(Length::Fill),
            container(text(""))
                .width(Length::Fill)
                .height(Length::Fixed(bar_height))
                .style(move |_| { container::Style::default().background(bar_color) }),
        ]
        .width(Length::FillPortion(1))
        .height(Length::Fixed(chart_body_height))
        .align_x(Horizontal::Center);

        bars = bars.push(
            mouse_area(bar)
                .on_enter(Message::ChartBarHovered(kind, index))
                .on_exit(Message::ChartBarHoverCleared(kind)),
        );
    }

    let range_text = format!("0 → {}", format_metric_value(metric, max_value));
    let first_label = sampled
        .first()
        .map(|item| item.label.clone())
        .unwrap_or_default();
    let last_label = sampled
        .last()
        .map(|item| item.label.clone())
        .unwrap_or_default();

    container(
        container(
            column![
                container(bars)
                    .width(Length::Fill)
                    .height(Length::Fixed(chart_body_height))
                    .padding([PLOT_PAD_V, PLOT_PAD_H])
                    .clip(true),
                container(
                    row![
                        container(text(first_label).size(12).font(Font::MONOSPACE))
                            .width(Length::FillPortion(1))
                            .align_x(Horizontal::Left),
                        container(text(range_text).size(12).font(Font::MONOSPACE))
                            .width(Length::FillPortion(1))
                            .align_x(Horizontal::Center),
                        container(text(last_label).size(12).font(Font::MONOSPACE))
                            .width(Length::FillPortion(1))
                            .align_x(Horizontal::Right),
                    ]
                    .width(Length::Fill)
                    .align_y(Vertical::Center)
                    .spacing(4),
                )
                .padding([6, PLOT_PAD_H]),
            ]
            .spacing(8),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(CHART_HEIGHT))
    .style(style_chart_canvas)
    .into()
}

fn sample_series_with_labels(labels: &[String], values: &[f64], limit: usize) -> Vec<SampledBar> {
    if values.len() <= limit {
        return values
            .iter()
            .enumerate()
            .map(|(index, value)| SampledBar {
                label: labels.get(index).cloned().unwrap_or_default(),
                value: *value,
            })
            .collect();
    }

    let step = values.len() as f64 / limit as f64;
    let mut out = Vec::with_capacity(limit);
    for i in 0..limit {
        let start = (i as f64 * step).floor() as usize;
        let mut end = ((i as f64 + 1.0) * step).ceil() as usize;
        end = end.min(values.len());
        if end <= start {
            continue;
        }
        let last_index = end - 1;
        out.push(SampledBar {
            label: labels.get(last_index).cloned().unwrap_or_default(),
            value: values.get(last_index).copied().unwrap_or(0.0),
        });
    }
    out
}

fn summary_cards(state: &GuiState) -> Element<'static, Message> {
    let (rows_count, total_tokens, total_cost) = if let Some(report) = &state.report {
        (
            report.daily.len().to_string(),
            format_u64(report.totals.total_tokens),
            format_usd(report.totals.cost_usd),
        )
    } else {
        ("-".to_string(), "-".to_string(), "-".to_string())
    };

    row![
        stat_card("Rows", rows_count),
        stat_card("Total Tokens", total_tokens),
        stat_card("Total Cost", total_cost),
    ]
    .spacing(10)
    .width(Length::Fill)
    .into()
}

fn stat_card(label: &'static str, value: String) -> Element<'static, Message> {
    container(
        column![
            text(label).size(13),
            text(value).size(24).font(Font::MONOSPACE),
        ]
        .spacing(4),
    )
    .padding([10, 12])
    .style(style_summary_card)
    .width(Length::FillPortion(1))
    .into()
}

fn period_button<'a>(
    label: &'a str,
    active: bool,
    message: Message,
) -> iced::widget::Button<'a, Message> {
    button(text(label).size(16))
        .style(if active {
            button::primary
        } else {
            button::secondary
        })
        .on_press(message)
}

fn build_table_header(layout: TableLayout) -> Element<'static, Message> {
    let row = match layout {
        TableLayout::Compact => row![
            table_cell("Date", 2, Horizontal::Left, false, true),
            column_separator(true),
            table_cell("Models", 5, Horizontal::Left, false, true),
            column_separator(true),
            table_cell("Total Tokens", 3, Horizontal::Right, true, true),
            column_separator(true),
            table_cell("Cost (USD)", 2, Horizontal::Right, true, true),
        ],
        TableLayout::Standard => row![
            table_cell("Date", 2, Horizontal::Left, false, true),
            column_separator(true),
            table_cell("Models", 4, Horizontal::Left, false, true),
            column_separator(true),
            table_cell("Input", 2, Horizontal::Right, true, true),
            column_separator(true),
            table_cell("Output", 2, Horizontal::Right, true, true),
            column_separator(true),
            table_cell("Total Tokens", 2, Horizontal::Right, true, true),
            column_separator(true),
            table_cell("Cost (USD)", 2, Horizontal::Right, true, true),
        ],
        TableLayout::Full => row![
            table_cell("Date", 2, Horizontal::Left, false, true),
            column_separator(true),
            table_cell("Models", 4, Horizontal::Left, false, true),
            column_separator(true),
            table_cell("Input", 2, Horizontal::Right, true, true),
            column_separator(true),
            table_cell("Output", 2, Horizontal::Right, true, true),
            column_separator(true),
            table_cell("Cache Create", 2, Horizontal::Right, true, true),
            column_separator(true),
            table_cell("Cache Read", 2, Horizontal::Right, true, true),
            column_separator(true),
            table_cell("Total Tokens", 2, Horizontal::Right, true, true),
            column_separator(true),
            table_cell("Cost (USD)", 2, Horizontal::Right, true, true),
        ],
    };

    container(row.spacing(0).width(Length::Fill))
        .padding([9, 10])
        .width(Length::Fill)
        .style(style_table_header)
        .into()
}

fn report_row(row: &DailyRow, layout: TableLayout) -> Element<'static, Message> {
    let line = match layout {
        TableLayout::Compact => row![
            table_cell_owned(row.date.clone(), 2, Horizontal::Left, false, false),
            column_separator(false),
            table_cell_owned(
                models_summary(&row.models, layout),
                5,
                Horizontal::Left,
                false,
                false
            ),
            column_separator(false),
            table_cell_owned(
                format_u64(row.totals.total_tokens),
                3,
                Horizontal::Right,
                true,
                false,
            ),
            column_separator(false),
            table_cell_owned(
                format_usd(row.totals.cost_usd),
                2,
                Horizontal::Right,
                true,
                false,
            ),
        ],
        TableLayout::Standard => row![
            table_cell_owned(row.date.clone(), 2, Horizontal::Left, false, false),
            column_separator(false),
            table_cell_owned(
                models_summary(&row.models, layout),
                4,
                Horizontal::Left,
                false,
                false
            ),
            column_separator(false),
            table_cell_owned(
                format_u64(row.totals.input_tokens),
                2,
                Horizontal::Right,
                true,
                false,
            ),
            column_separator(false),
            table_cell_owned(
                format_u64(row.totals.output_tokens),
                2,
                Horizontal::Right,
                true,
                false,
            ),
            column_separator(false),
            table_cell_owned(
                format_u64(row.totals.total_tokens),
                2,
                Horizontal::Right,
                true,
                false,
            ),
            column_separator(false),
            table_cell_owned(
                format_usd(row.totals.cost_usd),
                2,
                Horizontal::Right,
                true,
                false,
            ),
        ],
        TableLayout::Full => row![
            table_cell_owned(row.date.clone(), 2, Horizontal::Left, false, false),
            column_separator(false),
            table_cell_owned(
                models_summary(&row.models, layout),
                4,
                Horizontal::Left,
                false,
                false
            ),
            column_separator(false),
            table_cell_owned(
                format_u64(row.totals.input_tokens),
                2,
                Horizontal::Right,
                true,
                false,
            ),
            column_separator(false),
            table_cell_owned(
                format_u64(row.totals.output_tokens),
                2,
                Horizontal::Right,
                true,
                false,
            ),
            column_separator(false),
            table_cell_owned(
                format_u64(row.totals.cache_creation_input_tokens),
                2,
                Horizontal::Right,
                true,
                false,
            ),
            column_separator(false),
            table_cell_owned(
                format_u64(row.totals.cache_read_input_tokens),
                2,
                Horizontal::Right,
                true,
                false,
            ),
            column_separator(false),
            table_cell_owned(
                format_u64(row.totals.total_tokens),
                2,
                Horizontal::Right,
                true,
                false,
            ),
            column_separator(false),
            table_cell_owned(
                format_usd(row.totals.cost_usd),
                2,
                Horizontal::Right,
                true,
                false,
            ),
        ],
    };

    line.spacing(0)
        .align_y(Vertical::Center)
        .width(Length::Fill)
        .into()
}

fn report_totals_row(totals: &TokenCounts, layout: TableLayout) -> Element<'static, Message> {
    let line = match layout {
        TableLayout::Compact => row![
            table_cell_owned("TOTAL".to_string(), 2, Horizontal::Left, true, true),
            column_separator(true),
            table_cell_owned("-".to_string(), 5, Horizontal::Left, false, true),
            column_separator(true),
            table_cell_owned(
                format_u64(totals.total_tokens),
                3,
                Horizontal::Right,
                true,
                true,
            ),
            column_separator(true),
            table_cell_owned(
                format_usd(totals.cost_usd),
                2,
                Horizontal::Right,
                true,
                true,
            ),
        ],
        TableLayout::Standard => row![
            table_cell_owned("TOTAL".to_string(), 2, Horizontal::Left, true, true),
            column_separator(true),
            table_cell_owned("-".to_string(), 4, Horizontal::Left, false, true),
            column_separator(true),
            table_cell_owned(
                format_u64(totals.input_tokens),
                2,
                Horizontal::Right,
                true,
                true,
            ),
            column_separator(true),
            table_cell_owned(
                format_u64(totals.output_tokens),
                2,
                Horizontal::Right,
                true,
                true,
            ),
            column_separator(true),
            table_cell_owned(
                format_u64(totals.total_tokens),
                2,
                Horizontal::Right,
                true,
                true,
            ),
            column_separator(true),
            table_cell_owned(
                format_usd(totals.cost_usd),
                2,
                Horizontal::Right,
                true,
                true,
            ),
        ],
        TableLayout::Full => row![
            table_cell_owned("TOTAL".to_string(), 2, Horizontal::Left, true, true),
            column_separator(true),
            table_cell_owned("-".to_string(), 4, Horizontal::Left, false, true),
            column_separator(true),
            table_cell_owned(
                format_u64(totals.input_tokens),
                2,
                Horizontal::Right,
                true,
                true,
            ),
            column_separator(true),
            table_cell_owned(
                format_u64(totals.output_tokens),
                2,
                Horizontal::Right,
                true,
                true,
            ),
            column_separator(true),
            table_cell_owned(
                format_u64(totals.cache_creation_input_tokens),
                2,
                Horizontal::Right,
                true,
                true,
            ),
            column_separator(true),
            table_cell_owned(
                format_u64(totals.cache_read_input_tokens),
                2,
                Horizontal::Right,
                true,
                true,
            ),
            column_separator(true),
            table_cell_owned(
                format_u64(totals.total_tokens),
                2,
                Horizontal::Right,
                true,
                true,
            ),
            column_separator(true),
            table_cell_owned(
                format_usd(totals.cost_usd),
                2,
                Horizontal::Right,
                true,
                true,
            ),
        ],
    };

    line.spacing(0)
        .align_y(Vertical::Center)
        .width(Length::Fill)
        .into()
}

fn table_cell<'a>(
    value: &'a str,
    portion: u16,
    align: Horizontal,
    monospace: bool,
    emphasized: bool,
) -> Element<'a, Message> {
    let mut label = text(value).size(if emphasized { 16 } else { 15 });
    if monospace {
        label = label.font(Font::MONOSPACE);
    }

    container(label)
        .width(Length::FillPortion(portion))
        .align_x(align)
        .padding([2, 6])
        .into()
}

fn table_cell_owned(
    value: String,
    portion: u16,
    align: Horizontal,
    monospace: bool,
    emphasized: bool,
) -> Element<'static, Message> {
    let mut label = text(value).size(if emphasized { 16 } else { 15 });
    if monospace {
        label = label.font(Font::MONOSPACE);
    }

    container(label)
        .width(Length::FillPortion(portion))
        .align_x(align)
        .padding([1, 6])
        .into()
}

fn column_separator(emphasized: bool) -> Element<'static, Message> {
    container(
        text(if emphasized { "┃" } else { "│" })
            .font(Font::MONOSPACE)
            .size(if emphasized { 16 } else { 14 }),
    )
    .width(Length::Fixed(12.0))
    .align_x(Horizontal::Center)
    .align_y(Vertical::Center)
    .into()
}

impl TrendChart {
    fn new(
        title: &'static str,
        metric: TrendMetric,
        labels: Vec<String>,
        values: Vec<f64>,
        color: Color,
    ) -> Self {
        Self {
            title,
            metric,
            labels,
            values,
            color,
        }
    }

    fn empty_tokens() -> Self {
        Self::new(
            "Token Trend",
            TrendMetric::Tokens,
            Vec::new(),
            Vec::new(),
            Color::from_rgb8(0x2A, 0xC3, 0xDE),
        )
    }

    fn empty_cost() -> Self {
        Self::new(
            "Cost Trend (USD)",
            TrendMetric::CostUsd,
            Vec::new(),
            Vec::new(),
            Color::from_rgb8(0x9E, 0xCE, 0x6A),
        )
    }
}

fn compact_date_label(raw: &str) -> String {
    if raw.len() == 10 && raw.as_bytes().get(4) == Some(&b'-') {
        raw[5..].to_string()
    } else if (raw.len() == 7 && raw.as_bytes().get(4) == Some(&b'-')) || raw.contains("-W") {
        raw.to_string()
    } else if raw.len() > 10 {
        raw.chars().take(10).collect()
    } else {
        raw.to_string()
    }
}

fn format_metric_value(metric: TrendMetric, value: f64) -> String {
    match metric {
        TrendMetric::Tokens => format_u64(value.max(0.0).round() as u64),
        TrendMetric::CostUsd => format_usd(value.max(0.0)),
    }
}

fn with_alpha(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

fn models_summary(
    models: &std::collections::BTreeMap<String, TokenCounts>,
    layout: TableLayout,
) -> String {
    if models.is_empty() {
        return "-".to_string();
    }

    let mut sorted = models.iter().collect::<Vec<_>>();
    sorted.sort_by(|(a_name, a_counts), (b_name, b_counts)| {
        b_counts
            .total_tokens
            .cmp(&a_counts.total_tokens)
            .then_with(|| a_name.cmp(b_name))
    });

    let model_limit = layout.model_inline_limit();
    let mut items = sorted
        .into_iter()
        .take(model_limit)
        .map(|(name, _)| truncate_text(name, layout.model_inline_char_limit()))
        .collect::<Vec<_>>();

    if models.len() > model_limit {
        items.push(format!("+{}", models.len() - model_limit));
    }

    truncate_text(&items.join(" · "), layout.model_inline_char_limit())
}

fn table_layout_for_width(width_px: f32) -> TableLayout {
    let content_width = (width_px - 56.0).max(320.0);

    if content_width >= min_table_width(TableLayout::Full) {
        TableLayout::Full
    } else if content_width >= min_table_width(TableLayout::Standard) {
        TableLayout::Standard
    } else {
        TableLayout::Compact
    }
}

fn min_table_width(layout: TableLayout) -> f32 {
    // Approximate logical-pixel width needed for each layout:
    // total = portion_count * min_unit_width + separator_count * separator_width + safety_margin
    let (portion_count, separator_count, min_unit_width) = match layout {
        TableLayout::Compact => (12.0, 3.0, 76.0),
        TableLayout::Standard => (14.0, 5.0, 74.0),
        TableLayout::Full => (18.0, 7.0, 72.0),
    };

    portion_count * min_unit_width + separator_count * 12.0 + 20.0
}

fn truncate_text(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }

    let mut out = String::with_capacity(max_chars);
    for (idx, ch) in input.chars().enumerate() {
        if idx >= max_chars - 3 {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn normalize_optional_string(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn format_u64(value: u64) -> String {
    let raw = value.to_string();
    let mut out = String::with_capacity(raw.len() + (raw.len() / 3));
    let total = raw.len();
    for (idx, ch) in raw.chars().enumerate() {
        out.push(ch);
        let remain = total.saturating_sub(idx + 1);
        if remain > 0 && remain.is_multiple_of(3) {
            out.push(',');
        }
    }
    out
}

fn format_usd(value: f64) -> String {
    format!("${value:.2}")
}

fn style_root(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style::default().background(palette.background.base.color)
}

fn style_status_chip(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    let mut style = container::rounded_box(theme);
    style.background = Some(palette.background.weak.color.into());
    style.border.width = 1.0;
    style.border.color = palette.background.strong.color;
    style
}

fn style_controls_panel(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    let mut style = container::rounded_box(theme);
    style.background = Some(palette.secondary.weak.color.into());
    style.border.width = 1.0;
    style.border.color = palette.background.strong.color;
    style
}

fn style_summary_card(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    let mut style = container::rounded_box(theme);
    style.background = Some(palette.primary.weak.color.into());
    style.border.width = 1.0;
    style.border.color = palette.primary.strong.color;
    style
}

fn style_chart_panel(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    let mut style = container::rounded_box(theme);
    style.background = Some(palette.background.weak.color.into());
    style.border.width = 1.0;
    style.border.color = palette.background.strong.color;
    style
}

fn style_chart_canvas(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    let mut style = container::rounded_box(theme);
    style.background = Some(palette.background.base.color.into());
    style.border.width = 1.0;
    style.border.color = palette.background.strong.color;
    style
}

fn style_table_header(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    let mut style = container::rounded_box(theme);
    style.background = Some(palette.primary.strong.color.into());
    style.text_color = Some(palette.primary.strong.text);
    style.border.width = 1.0;
    style.border.color = palette.primary.base.color;
    style
}

fn style_data_row(theme: &Theme, idx: usize) -> container::Style {
    let palette = theme.extended_palette();
    let mut style = container::rounded_box(theme);
    style.background = Some(
        if idx.is_multiple_of(2) {
            palette.background.base.color
        } else {
            palette.background.weak.color
        }
        .into(),
    );
    style.border.width = 1.0;
    style.border.color = palette.background.strong.color;
    style
}

fn style_total_row(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    let mut style = container::rounded_box(theme);
    style.background = Some(palette.success.weak.color.into());
    style.text_color = Some(palette.success.weak.text);
    style.border.width = 1.0;
    style.border.color = palette.success.base.color;
    style
}

fn style_placeholder(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    let mut style = container::rounded_box(theme);
    style.background = Some(palette.background.weak.color.into());
    style.border.width = 1.0;
    style.border.color = palette.background.strong.color;
    style
}

fn style_error_panel(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    let mut style = container::rounded_box(theme);
    style.background = Some(palette.danger.weak.color.into());
    style.text_color = Some(palette.danger.weak.text);
    style.border.width = 1.0;
    style.border.color = palette.danger.base.color;
    style
}

fn report_scroll_id() -> scrollable::Id {
    scrollable::Id::new("report-scroll")
}
