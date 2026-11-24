use std::{
    net::IpAddr,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

use eframe::egui::{self, Color32, RichText};

use crate::{
    discovery::{self, Station},
    package::Source,
    station::{self, Address, Installed, Model},
    wake_catalog::{Catalog, Runtime},
};

const PAPER: Color32 = Color32::WHITE;
const SURFACE: Color32 = Color32::from_rgb(246, 250, 247);
const TEXT: Color32 = Color32::from_rgb(23, 40, 32);
const MUTED: Color32 = Color32::from_rgb(73, 100, 86);
const EDGE: Color32 = Color32::from_rgb(220, 232, 223);
const ACCENT: Color32 = Color32::from_rgb(23, 102, 72);
const ACCENT_HOVER: Color32 = Color32::from_rgb(18, 86, 59);
const ACTIVE: Color32 = Color32::from_rgb(220, 239, 226);
const ERROR: Color32 = Color32::from_rgb(155, 53, 42);

#[derive(Default)]
pub(crate) struct InstallerWindow {
    stations: Vec<Station>,
    selected: Option<IpAddr>,
    search: Option<StationSearch>,
    search_error: Option<String>,
    manual: bool,
    manual_address: String,
    focus_address: bool,
    advanced_open: bool,
    local_build: Option<PathBuf>,
    adb: Option<PathBuf>,
    model: Model,
    catalog: Option<Catalog>,
    catalog_worker: Option<Receiver<Result<Catalog, String>>>,
    catalog_error: Option<String>,
    worker: Option<Receiver<Event>>,
    target: Option<InstallTarget>,
    status: String,
    log: Vec<String>,
    log_open: bool,
    attempt: u64,
    failure: Option<String>,
    installed: Option<Installed>,
}

struct InstallTarget {
    name: String,
    address: String,
}

enum Event {
    Log(String),
    Done(Result<Installed, String>),
}

enum SearchEvent {
    Stations(Vec<Station>),
    Done(anyhow::Result<()>),
}

struct StationSearch {
    events: Receiver<SearchEvent>,
    found: Vec<Station>,
}

pub(crate) fn configure(ctx: &egui::Context) {
    let mut style = egui::Style {
        visuals: egui::Visuals::light(),
        ..Default::default()
    };
    style.visuals.panel_fill = SURFACE;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.extreme_bg_color = PAPER;
    style.visuals.text_edit_bg_color = Some(SURFACE);
    style.visuals.selection.bg_fill = ACTIVE;
    style.visuals.selection.stroke.color = ACCENT;
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.widgets.noninteractive.bg_stroke.color = EDGE;
    style.visuals.widgets.inactive.weak_bg_fill = SURFACE;
    style.visuals.widgets.hovered.weak_bg_fill = ACTIVE;
    style.visuals.widgets.active.weak_bg_fill = ACTIVE;
    style.spacing.item_spacing = egui::vec2(9.0, 9.0);
    style.spacing.button_padding = egui::vec2(13.0, 7.0);
    style.spacing.interact_size.y = 33.0;
    for (size, key) in [
        (15.0, egui::TextStyle::Body),
        (15.0, egui::TextStyle::Button),
        (13.0, egui::TextStyle::Small),
        (13.0, egui::TextStyle::Monospace),
    ] {
        style
            .text_styles
            .get_mut(&key)
            .expect("egui default text style")
            .size = size;
    }
    ctx.set_style(style);
}

impl eframe::App for InstallerWindow {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        self.poll_search();
        self.poll_catalog();
        let busy = self.worker.is_some();
        if busy && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        self.render(ctx, busy);
    }
}

fn header(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(43.0, 43.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 13, ACTIVE);
        let left = rect.center() - egui::vec2(7.0, 0.0);
        let right = rect.center() + egui::vec2(7.0, 0.0);
        let center = rect.center();
        painter.add(egui::epaint::QuadraticBezierShape::from_points_stroke(
            [
                center - egui::vec2(3.0, 0.0),
                center - egui::vec2(0.0, 2.6),
                center + egui::vec2(3.0, 0.0),
            ],
            false,
            Color32::TRANSPARENT,
            egui::Stroke::new(1.5_f32, ACCENT),
        ));
        for center in [left, right] {
            painter.circle(
                center,
                4.8,
                Color32::from_rgb(108, 201, 145),
                egui::Stroke::new(1.4_f32, ACCENT),
            );
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new("Установщик Goodwin").size(26.0).strong());
            ui.label(RichText::new("Яндекс Станция Мини 3").color(MUTED));
        });
    });
}

fn card(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(PAPER)
        .stroke(egui::Stroke::new(1.0_f32, EDGE))
        .corner_radius(12)
        .inner_margin(15)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            body(ui);
        });
}

fn secondary_action(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, EDGE);
        widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, ACCENT);
        ui.add(
            egui::Button::selectable(selected, RichText::new(label).color(ACCENT)).corner_radius(8),
        )
    })
    .inner
}

fn log_toggle(ui: &mut egui::Ui, open: bool) -> egui::Response {
    let chevron = egui::Id::new("log_chevron");
    let button = egui::Button::new((
        egui::Atom::custom(chevron, egui::Vec2::splat(12.0)),
        "Журнал установки",
    ))
    .frame_when_inactive(false)
    .corner_radius(8)
    .atom_ui(ui);
    if let Some(rect) = button.rect(chevron) {
        let center = rect.center();
        let points = if open {
            [
                egui::vec2(-3.5, -1.5),
                egui::vec2(0.0, 2.0),
                egui::vec2(3.5, -1.5),
            ]
        } else {
            [
                egui::vec2(-1.5, -3.5),
                egui::vec2(2.0, 0.0),
                egui::vec2(-1.5, 3.5),
            ]
        };
        ui.painter().line(
            points.map(|point| center + point).to_vec(),
            egui::Stroke::new(1.5_f32, MUTED),
        );
    }
    button.response
}

fn station_row(ui: &mut egui::Ui, station: &Station, selected: bool) -> egui::Response {
    let marker = egui::Id::new("selection");
    let address = station.address.to_string();
    let response = egui::Button::selectable(
        selected,
        (
            egui::Atom::custom(marker, egui::Vec2::splat(16.0)),
            RichText::new(&station.name).strong(),
        ),
    )
    .right_text(RichText::new(&address).color(MUTED).monospace().small())
    .min_size(egui::vec2(ui.available_width(), 38.0))
    .corner_radius(8)
    .truncate()
    .atom_ui(ui);
    if let Some(rect) = response.rect(marker)
        && ui.is_rect_visible(rect)
    {
        let center = rect.center();
        if selected {
            ui.painter().circle_filled(center, 7.0, ACCENT);
            ui.painter().line(
                vec![
                    center + egui::vec2(-3.4, 0.0),
                    center + egui::vec2(-1.1, 2.3),
                    center + egui::vec2(3.5, -2.3),
                ],
                egui::Stroke::new(1.4_f32, PAPER),
            );
        } else {
            ui.painter()
                .circle_stroke(center, 6.5, egui::Stroke::new(1.0_f32, MUTED));
        }
    }
    response
        .response
        .on_hover_text(format!("{} · {address}", station.name))
}

impl InstallerWindow {
    pub(crate) fn new(ctx: &egui::Context) -> Self {
        let mut window = Self::default();
        window.start_search(ctx);
        window.load_catalog(ctx);
        window
    }

    fn render(&mut self, ctx: &egui::Context, busy: bool) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(SURFACE).inner_margin(24))
            .show(ctx, |ui| {
                header(ui);
                ui.add_space(10.0);

                if self.target.is_some() {
                    self.installation(ui, busy);
                    if self.target.is_some() {
                        self.installation_log(ui);
                    }
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("setup")
                        .show(ui, |ui| {
                            self.station_fields(ui);
                            if let Some(error) = &self.failure {
                                ui.label(RichText::new(error).color(ERROR));
                            }
                        });
                }
            });
        if self.advanced_open {
            self.advanced(ctx);
        }
    }

    fn station_fields(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.add_enabled_ui(!self.advanced_open, |ui| {
                self.station_picker(ui);
                ui.separator();
                self.wake_model_picker(ui);
            });
            ui.separator();
            self.controls(ui);
        });
    }

    fn wake_model_picker(&mut self, ui: &mut egui::Ui) {
        const KEEP: &str = "Текущая / «Гудвин» Standard при установке";
        let label = |model: &crate::wake_catalog::Selection| {
            let tier = match model.runtime() {
                Runtime::MicroTflite => "Micro INT8",
                Runtime::Micro => "Micro",
                _ => "Standard",
            };
            format!("{} · {tier}", model.wake_word)
        };
        ui.horizontal_wrapped(|ui| {
            ui.label("Модель активации");
            let selected = self.catalog.as_ref().and_then(|catalog| {
                catalog
                    .models
                    .iter()
                    .find(|model| self.model == Model::Id(model.selection.id.clone()))
            });
            egui::ComboBox::from_id_salt("wake_model")
                .selected_text(
                    selected.map_or_else(|| KEEP.into(), |model| label(&model.selection)),
                )
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.model, Model::Keep, KEEP);
                    if let Some(catalog) = &self.catalog {
                        for model in &catalog.models {
                            ui.selectable_value(
                                &mut self.model,
                                Model::Id(model.selection.id.clone()),
                                label(&model.selection),
                            );
                        }
                    }
                });
            if self.catalog_worker.is_some() {
                ui.spinner();
                ui.label("Загружаем список…");
            } else if let Some(error) = &self.catalog_error {
                ui.label(RichText::new("Список моделей недоступен").color(ERROR))
                    .on_hover_text(error);
                if ui.button("Повторить").clicked() {
                    self.load_catalog(ui.ctx());
                }
            }
        });
    }

    fn load_catalog(&mut self, ctx: &egui::Context) {
        let (send, receive) = mpsc::channel();
        self.catalog_worker = Some(receive);
        self.catalog_error = None;
        let ctx = ctx.clone();
        thread::spawn(move || {
            let catalog = Catalog::latest(Runtime::available(true))
                .map_err(|error| format!("{error:#}"))
                .and_then(|catalog| {
                    if catalog.models.is_empty() {
                        Err("В релизе нет совместимых моделей".into())
                    } else {
                        Ok(catalog)
                    }
                });
            let _ = send.send(catalog);
            ctx.request_repaint();
        });
    }

    fn poll_catalog(&mut self) {
        let Some(worker) = &self.catalog_worker else {
            return;
        };
        match worker.try_recv() {
            Ok(Ok(catalog)) => self.catalog = Some(catalog),
            Ok(Err(error)) => self.catalog_error = Some(error),
            Err(TryRecvError::Disconnected) => {
                self.catalog_error = Some("Не удалось получить список моделей".into())
            }
            Err(TryRecvError::Empty) => return,
        }
        self.catalog_worker = None;
    }

    fn station_picker(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Станции в локальной сети")
                    .size(17.0)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let slot = egui::vec2(100.0, 33.0);
                if self.search.is_some() {
                    ui.allocate_ui_with_layout(
                        slot,
                        egui::Layout::left_to_right(egui::Align::Center)
                            .with_main_align(egui::Align::Center),
                        |ui| {
                            ui.add(egui::Spinner::new().size(13.0).color(MUTED));
                            ui.label(RichText::new("Поиск…").color(MUTED).small());
                        },
                    );
                } else if ui
                    .add_sized(slot, egui::Button::new("Обновить").corner_radius(8))
                    .clicked()
                {
                    self.start_search(ui.ctx());
                }
            });
        });

        let stations = self.visible_stations();
        if stations.is_empty() {
            ui.allocate_ui(egui::vec2(ui.available_width(), 38.0), |ui| {
                if let Some(error) = &self.search_error {
                    ui.label(RichText::new("Не удалось запустить поиск.").color(MUTED))
                        .on_hover_text(error);
                } else if self.search.is_none() {
                    ui.label(RichText::new("Станции Мини 3 не найдены.").color(MUTED));
                    ui.label(
                        RichText::new(
                            "Проверьте, что компьютер и Станция подключены к одной сети.",
                        )
                        .color(MUTED)
                        .small(),
                    );
                }
            });
        } else {
            let choice = egui::ScrollArea::vertical()
                .id_salt("stations")
                .max_height(208.0)
                .show(ui, |ui| {
                    let mut choice = None;
                    for station in stations {
                        let selected = !self.manual && self.selected == Some(station.address);
                        let response = ui
                            .push_id(station.address, |ui| station_row(ui, station, selected))
                            .inner;
                        if response.clicked() {
                            choice = Some(station.address);
                        }
                    }
                    choice
                })
                .inner;
            if let Some(address) = choice {
                self.selected = Some(address);
                self.manual = false;
            }
        }

        ui.horizontal_wrapped(|ui| {
            if secondary_action(ui, "Указать адрес вручную", self.manual).clicked()
            {
                self.manual = !self.manual;
                if self.manual {
                    if self.manual_address.is_empty()
                        && let Some(ip) = self.selected
                    {
                        self.manual_address = match ip {
                            IpAddr::V4(ip) => ip.to_string(),
                            IpAddr::V6(ip) => format!("[{ip}]"),
                        };
                    }
                    self.focus_address = true;
                }
            }
            if secondary_action(ui, "Дополнительные настройки", false).clicked()
            {
                self.advanced_open = true;
            }
        });
        if self.manual {
            let input = ui
                .add(
                    egui::TextEdit::singleline(&mut self.manual_address)
                        .desired_width(f32::INFINITY)
                        .hint_text("IP-адрес или имя, например 192.168.1.42")
                        .margin(egui::vec2(10.0, 8.0)),
                )
                .on_hover_text(
                    "Порт ADB по умолчанию — 5555. Другой порт можно указать после двоеточия.",
                );
            if std::mem::take(&mut self.focus_address) {
                input.request_focus();
            }
        }
    }

    fn visible_stations(&self) -> &[Station] {
        if self.stations.is_empty()
            && let Some(search) = &self.search
        {
            &search.found
        } else {
            &self.stations
        }
    }

    fn advanced(&mut self, ctx: &egui::Context) {
        let modal = egui::Modal::new(egui::Id::new("advanced"))
            .frame(
                egui::Frame::popup(&ctx.style())
                    .fill(PAPER)
                    .corner_radius(12)
                    .inner_margin(20),
            )
            .backdrop_color(Color32::from_black_alpha(64))
            .show(ctx, |ui| {
                ui.set_width(390.0);
                ui.label(
                    RichText::new("Дополнительные настройки")
                        .size(20.0)
                        .strong(),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Выбрать ADB…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_title("Выберите исполняемый файл ADB")
                            .pick_file()
                    {
                        self.adb = Some(path);
                    }
                    if self.adb.is_some() && ui.button("Сбросить").clicked() {
                        self.adb = None;
                    }
                    let label = self.adb.as_deref().map_or_else(
                        || "Рядом с приложением или в PATH".into(),
                        |path| path.display().to_string(),
                    );
                    ui.add(egui::Label::new(RichText::new(&label).color(MUTED).small()).truncate())
                        .on_hover_text(label);
                });
                ui.separator();
                ui.label(RichText::new("Локальная сборка").strong());
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Выбрать архив…").clicked()
                        && let Some(path) = self
                            .local_dialog()
                            .set_title("Выберите архив Goodwin")
                            .add_filter("Архив Goodwin", &["gz"])
                            .pick_file()
                    {
                        self.local_build = Some(path);
                    }
                    if ui.button("Выбрать папку…").clicked()
                        && let Some(path) = self
                            .local_dialog()
                            .set_title("Выберите папку со сборкой Goodwin")
                            .pick_folder()
                    {
                        self.local_build = Some(path);
                    }
                    if self.local_build.is_some() && ui.button("Сбросить").clicked() {
                        self.local_build = None;
                    }
                });
                if let Some(path) = &self.local_build {
                    let label = path.display().to_string();
                    ui.add(egui::Label::new(RichText::new(&label).color(MUTED).small()).truncate())
                        .on_hover_text(label);
                } else {
                    ui.label(
                        RichText::new("Для тестов подойдёт архив .tar.gz или папка со сборкой.")
                            .color(MUTED)
                            .small(),
                    );
                }
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.button("Готово").clicked()
                })
                .inner
            });
        if modal.inner || modal.should_close() {
            self.advanced_open = false;
        }
    }

    fn local_dialog(&self) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new();
        match self.local_build.as_deref() {
            Some(path) if path.is_dir() => dialog.set_directory(path),
            Some(path) if path.is_file() => {
                dialog.set_directory(path.parent().unwrap_or(Path::new(".")))
            }
            _ => dialog,
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(3.0);
        ui.horizontal_wrapped(|ui| {
            let ready = !self.advanced_open
                && if self.manual {
                    !self.manual_address.trim().is_empty()
                } else {
                    let stations = self
                        .search
                        .as_ref()
                        .map_or(self.stations.as_slice(), |search| search.found.as_slice());
                    stations
                        .iter()
                        .any(|station| Some(station.address) == self.selected)
                };
            let install = egui::Button::new(
                RichText::new("Установить или обновить")
                    .strong()
                    .color(if ready { PAPER } else { MUTED }),
            )
            .min_size(egui::vec2(0.0, 39.0))
            .corner_radius(8);
            let pressed = ui
                .scope(|ui| {
                    let widgets = &mut ui.visuals_mut().widgets;
                    widgets.noninteractive.weak_bg_fill = SURFACE;
                    widgets.inactive.weak_bg_fill = ACCENT;
                    widgets.hovered.weak_bg_fill = ACCENT_HOVER;
                    widgets.active.weak_bg_fill = ACCENT_HOVER;
                    ui.add_enabled(ready, install)
                })
                .inner
                .clicked();
            if pressed {
                self.start(ui.ctx());
            }
            if self.local_build.is_some() {
                ui.label(
                    RichText::new("Будет использована локальная сборка")
                        .color(MUTED)
                        .small(),
                );
            }
        });
        ui.label(
            RichText::new("Для установки нужен сетевой ADB с правами root.")
                .color(MUTED)
                .small(),
        );
    }

    fn installation(&mut self, ui: &mut egui::Ui, busy: bool) {
        let Some(target) = &self.target else { return };
        let title = if busy {
            RichText::new("Устанавливаем Goodwin")
        } else if let Some(installed) = &self.installed {
            RichText::new(if installed.updated {
                "Goodwin обновлён"
            } else {
                "Goodwin установлен"
            })
            .color(ACCENT)
        } else {
            RichText::new("Установка не завершилась").color(ERROR)
        };

        let mut back = false;
        card(ui, |ui| {
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.spacing_mut().interact_size.y = 21.0;
                ui.label(title.size(19.0).strong());
                if busy {
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::Spinner::new().size(14.0).color(MUTED));
                        ui.label(RichText::new(&self.status).color(TEXT));
                    });
                }

                egui::Frame::new()
                    .fill(SURFACE)
                    .inner_margin(9)
                    .corner_radius(8)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&target.name).strong()).truncate(),
                            )
                            .on_hover_text(&target.name);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&target.address).monospace().color(MUTED),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&target.address);
                        });
                    });
            });

            if !busy {
                if let Some(error) = &self.failure {
                    let summary = error.lines().next().unwrap_or(error);
                    ui.add(egui::Label::new(RichText::new(summary).color(ERROR)).truncate())
                        .on_hover_text(summary);
                }
                if let Some(installed) = &self.installed {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&installed.panel).monospace().color(MUTED),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&installed.panel);
                        ui.add(
                            egui::Label::new(RichText::new(&installed.source).color(MUTED).small())
                                .truncate(),
                        )
                        .on_hover_text(&installed.source);
                    });
                    if !installed.updated {
                        ui.label("Создайте пароль при первом входе в панель.");
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    if let Some(installed) = &self.installed
                        && ui
                            .button("Открыть панель")
                            .on_hover_text(&installed.panel)
                            .clicked()
                        && webbrowser::open(&installed.panel).is_err()
                    {
                        ui.ctx().copy_text(installed.panel.clone());
                        self.failure =
                            Some("Не удалось открыть браузер. Адрес панели скопирован.".into());
                    }
                    back = secondary_action(ui, "К выбору Станции", false).clicked();
                });
            }
        });
        if back {
            self.target = None;
            self.installed = None;
            self.failure = None;
            ui.ctx().request_repaint();
        }
    }

    fn installation_log(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                if log_toggle(ui, self.log_open).clicked() {
                    self.log_open = !self.log_open;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(!self.log.is_empty(), |ui| {
                        if secondary_action(ui, "Скопировать", false).clicked() {
                            ui.ctx().copy_text(self.log.join("\n"));
                        }
                    });
                });
            });
            if self.log_open {
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt(("installation_log", self.attempt))
                    .auto_shrink([false, false])
                    .max_height(ui.available_height())
                    .min_scrolled_height(0.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for line in &self.log {
                            ui.add(egui::Label::new(RichText::new(line).monospace()).wrap());
                        }
                    });
            }
        });
    }

    fn start(&mut self, ctx: &egui::Context) {
        self.failure = None;
        self.installed = None;
        self.log.clear();
        let target = if self.manual {
            Address::parse(&self.manual_address)
        } else {
            let Some(ip) = self.selected else { return };
            Ok(Address::from_ip(ip))
        };
        let address = match target {
            Ok(address) => address,
            Err(error) => {
                self.failure = Some(error.to_string());
                return;
            }
        };
        let name = if self.manual {
            "Станция".into()
        } else {
            self.visible_stations()
                .iter()
                .find(|station| Some(station.address) == self.selected)
                .map_or_else(|| "Станция Мини 3".into(), |station| station.name.clone())
        };
        self.target = Some(InstallTarget {
            name,
            address: address.serial(),
        });
        self.attempt = self.attempt.wrapping_add(1);
        let source = self
            .local_build
            .clone()
            .map_or(Source::Latest, Source::Local);
        let adb = self.adb.clone().unwrap_or_else(station::default_adb);
        let model = self.model.clone();
        let catalog = self.catalog.clone();
        let ctx = ctx.clone();
        let (send, receive) = mpsc::channel();
        self.status = "Подключаемся к Станции…".into();
        self.worker = Some(receive);
        ctx.request_repaint();
        thread::spawn(move || {
            let result = station::install(&adb, address, source, model, catalog.as_ref(), |line| {
                let _ = send.send(Event::Log(line));
                ctx.request_repaint();
            });
            let _ = send.send(Event::Done(result.map_err(|error| format!("{error:#}"))));
            ctx.request_repaint();
        });
    }

    fn start_search(&mut self, ctx: &egui::Context) {
        self.search_error = None;
        let (send, receive) = mpsc::channel();
        self.search = Some(StationSearch {
            events: receive,
            found: Vec::new(),
        });
        let ctx = ctx.clone();
        thread::spawn(move || {
            let result = discovery::search(|stations| {
                let _ = send.send(SearchEvent::Stations(stations));
                ctx.request_repaint();
            });
            let _ = send.send(SearchEvent::Done(result));
            ctx.request_repaint();
        });
    }

    fn poll_search(&mut self) {
        let Some(search) = &mut self.search else {
            return;
        };
        loop {
            match search.events.try_recv() {
                Ok(SearchEvent::Stations(stations)) => search.found = stations,
                Ok(SearchEvent::Done(result)) => {
                    if result.is_ok() {
                        self.stations = std::mem::take(&mut search.found);
                    }
                    self.search = None;
                    self.search_error = result.err().map(|error| format!("{error:#}"));
                    if !self
                        .stations
                        .iter()
                        .any(|station| Some(station.address) == self.selected)
                    {
                        self.selected = match self.stations.as_slice() {
                            [station] => Some(station.address),
                            _ => None,
                        };
                    }
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.search = None;
                    self.search_error = Some("Поиск прервался.".into());
                    break;
                }
            }
        }
    }

    fn poll(&mut self) {
        let Some(worker) = &self.worker else { return };
        loop {
            match worker.try_recv() {
                Ok(Event::Log(line)) => {
                    if line.ends_with('…') {
                        self.status.clone_from(&line);
                    }
                    self.log.push(line);
                }
                Ok(Event::Done(result)) => {
                    match result {
                        Ok(installed) => self.installed = Some(installed),
                        Err(error) => self.installation_failed(error),
                    }
                    self.worker = None;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.installation_failed("Установка прервалась.".into());
                    break;
                }
            }
        }
    }

    fn installation_failed(&mut self, error: String) {
        self.log.extend(error.lines().map(str::to_owned));
        self.log_open = true;
        self.failure = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible_text(output: &egui::FullOutput, text: &str) -> Option<(egui::Rect, egui::Rect)> {
        output.shapes.iter().find_map(|shape| {
            let egui::Shape::Text(label) = &shape.shape else {
                return None;
            };
            let bounds = label.visual_bounding_rect();
            (label.galley.text() == text && shape.clip_rect.contains_rect(bounds))
                .then_some((bounds, shape.clip_rect))
        })
    }

    #[test]
    fn scrolling_the_installation_log_keeps_the_header_and_stage_in_place() {
        let ctx = egui::Context::default();
        configure(&ctx);
        let bounds = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(460.0, 370.0));
        let mut window = InstallerWindow {
            target: Some(InstallTarget {
                name: "Станция Мини 3".into(),
                address: "192.168.1.42:5555".into(),
            }),
            status: "Отправляем пакет на Станцию…".into(),
            log_open: true,
            log: (0..60).map(|line| format!("Log entry {line:02}")).collect(),
            ..Default::default()
        };
        let frame = |window: &mut InstallerWindow, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(bounds),
                    events,
                    ..Default::default()
                },
                |ctx| window.render(ctx, true),
            )
        };
        frame(&mut window, vec![]);
        let at_end = frame(&mut window, vec![]);
        let header = visible_text(&at_end, "Установщик Goodwin").unwrap().0;
        let stage = visible_text(&at_end, "Отправляем пакет на Станцию…")
            .unwrap()
            .0;
        let (_, log_clip) = visible_text(&at_end, "Log entry 59").unwrap_or_else(|| {
            let labels: Vec<_> = at_end
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(label) => Some((
                        label.galley.text(),
                        label.visual_bounding_rect(),
                        shape.clip_rect,
                    )),
                    _ => None,
                })
                .collect();
            panic!("Log tail is outside the view: {labels:?}");
        });
        assert!(visible_text(&at_end, "192.168.1.42:5555").is_some());
        assert!(visible_text(&at_end, "Установить или обновить").is_none());
        assert!(log_clip.width() > bounds.width() * 0.75);
        assert!(bounds.contains_rect(log_clip));

        window.log.push("Log entry 60".into());
        frame(&mut window, vec![]);
        let following = frame(&mut window, vec![]);
        assert!(visible_text(&following, "Log entry 60").is_some());

        frame(
            &mut window,
            vec![
                egui::Event::PointerMoved(log_clip.center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 240.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let earlier = frame(&mut window, vec![]);
        assert!(visible_text(&earlier, "Log entry 60").is_none());
        assert_eq!(
            visible_text(&earlier, "Установщик Goodwin").unwrap().0,
            header
        );
        assert_eq!(
            visible_text(&earlier, "Отправляем пакет на Станцию…")
                .unwrap()
                .0,
            stage
        );

        window.log.push("Log entry 61".into());
        frame(&mut window, vec![]);
        let new_entry = frame(&mut window, vec![]);
        assert!(visible_text(&new_entry, "Log entry 61").is_none());
    }
}
