use futures_util::future::join_all;

use std::path::{Path, PathBuf};

use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, adw, adw::prelude::*, gtk};

use crate::{
    stages::identify::{IdentifiedSource, identify_source},
    vndb::{VnSummary, VndbClient, VndbError},
};

const COVER_WIDTH: i32 = 72;
const COVER_HEIGHT: i32 = 108;
const CARD_COVER_WIDTH: i32 = 144;
const CARD_COVER_HEIGHT: i32 = 216;

pub struct App {
    page: Page,
    games: Vec<LibraryGame>,
    library_error: Option<String>,
    query: String,
    search: SearchState,
    vndb: VndbClient,
    sender: ComponentSender<App>,
    games_grid: gtk::FlowBox,
    query_entry: gtk::Entry,
    results_list: gtk::ListBox,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Library,
    Search,
}

impl Page {
    fn name(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Search => "search",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Library => "Library",
            Self::Search => "Search VNDB",
        }
    }
}

#[derive(Clone, Debug)]
struct LibraryGame {
    title: String,
    origin: GameOrigin,
    thumbnail: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
enum GameOrigin {
    Local(IdentifiedSource),
    Vndb(VnSummary),
}

#[derive(Debug)]
pub enum AppMsg {
    AddLocal,
    ChooseSourceFile,
    ChooseSourceFolder,
    SourceSelected(PathBuf),
    ShowSearch,
    QueryChanged(String),
    SearchVndb,
    SelectMatch(SearchResultWithThumbnail),
    ShowLibrary,
}

#[derive(Debug)]
pub enum CommandOutput {
    SearchFinished {
        query: String,
        result: Result<SearchDisplayResults, VndbError>,
    },
}

enum SearchState {
    Idle,
    Loading {
        query: String,
    },
    Loaded {
        query: String,
        count: usize,
        more: bool,
    },
    Failed {
        query: String,
        message: String,
    },
}

#[derive(Debug)]
pub struct SearchDisplayResults {
    entries: Vec<SearchResultWithThumbnail>,
    more: bool,
}

#[derive(Clone, Debug)]
pub struct SearchResultWithThumbnail {
    summary: VnSummary,
    thumbnail: Option<Vec<u8>>,
}

#[relm4::component(pub)]
impl Component for App {
    type Init = ();
    type Input = AppMsg;
    type Output = ();
    type CommandOutput = CommandOutput;

    view! {
        #[root]
        adw::ApplicationWindow {
            set_title: Some("ryoiki"),
            set_default_size: (900, 680),

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,

                adw::HeaderBar {
                    pack_start = &gtk::Button {
                        set_icon_name: "go-previous-symbolic",
                        set_tooltip_text: Some("Back to library"),
                        #[watch]
                        set_visible: model.page == Page::Search,
                        connect_clicked => AppMsg::ShowLibrary,
                    },

                    #[wrap(Some)]
                    set_title_widget = &gtk::Label {
                        #[watch]
                        set_label: model.page.title(),
                        add_css_class: "heading",
                    },

                    pack_end = &gtk::MenuButton {
                        set_icon_name: "list-add-symbolic",
                        set_tooltip_text: Some("Add game"),
                        #[watch]
                        set_visible: model.page == Page::Library,

                        #[wrap(Some)]
                        set_popover = &gtk::Popover {
                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_spacing: 6,
                                set_margin_all: 6,

                                gtk::Button {
                                    set_label: "Archive or folder",
                                    add_css_class: "flat",
                                    connect_clicked => AppMsg::AddLocal,
                                },

                                gtk::Separator {},

                                gtk::Button {
                                    set_label: "Search VNDB",
                                    add_css_class: "flat",
                                    connect_clicked => AppMsg::ShowSearch,
                                },
                            },
                        },
                    },
                },

                append: &page_stack,
            },
        },

        page_stack = &gtk::Stack {
            set_vexpand: true,
            set_transition_type: gtk::StackTransitionType::Crossfade,
            set_transition_duration: 150,
            add_named: (&library_page, Some("library")),
            add_named: (&search_page, Some("search")),
            #[watch]
            set_visible_child_name: model.page.name(),
        },

        library_page = &gtk::ScrolledWindow {
            set_hscrollbar_policy: gtk::PolicyType::Never,

            #[wrap(Some)]
            set_child = &gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 18,
                set_margin_all: 24,

                adw::StatusPage {
                    set_icon_name: Some("folder-documents-symbolic"),
                    set_title: "No games yet",
                    set_description: Some("Press + to add an archive, a folder, or a VNDB title."),
                    #[watch]
                    set_visible: model.games.is_empty(),
                },

                gtk::Label {
                    #[watch]
                    set_label: model.library_error.as_deref().unwrap_or(""),
                    #[watch]
                    set_visible: model.library_error.is_some(),
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_selectable: true,
                },

                append: &model.games_grid,
            },
        },

        search_page = &gtk::ScrolledWindow {
            set_hscrollbar_policy: gtk::PolicyType::Never,

            #[wrap(Some)]
            set_child = &adw::Clamp {
                set_hexpand: true,
                set_maximum_size: 720,
                set_tightening_threshold: 520,

                #[wrap(Some)]
                set_child = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 18,
                    set_margin_all: 32,

                    gtk::Label {
                        set_label: "Add from VNDB",
                        set_xalign: 0.0,
                        add_css_class: "title-1",
                    },

                    gtk::Label {
                        set_label: "Search and add a title. No local files will be installed.",
                        set_xalign: 0.0,
                        set_wrap: true,
                        add_css_class: "dim-label",
                    },

                    gtk::Box {
                        set_spacing: 8,

                        append: &model.query_entry,

                        gtk::Button {
                            set_label: "Search",
                            add_css_class: "suggested-action",
                            #[watch]
                            set_sensitive: model.can_search(),
                            connect_clicked => AppMsg::SearchVndb,
                        },

                        gtk::Spinner {
                            #[watch]
                            set_spinning: model.is_searching(),
                            #[watch]
                            set_visible: model.is_searching(),
                        },
                    },

                    gtk::Separator {},

                    gtk::Label {
                        #[watch]
                        set_label: &model.search_text(),
                        set_xalign: 0.0,
                        set_yalign: 0.0,
                        set_wrap: true,
                    },

                    append: &model.results_list,
                },
            },
        },
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let games_grid = gtk::FlowBox::new();
        games_grid.set_selection_mode(gtk::SelectionMode::None);
        games_grid.set_homogeneous(true);
        games_grid.set_min_children_per_line(1);
        games_grid.set_max_children_per_line(8);
        games_grid.set_column_spacing(12);
        games_grid.set_row_spacing(12);
        games_grid.set_valign(gtk::Align::Start);
        games_grid.set_hexpand(true);
        games_grid.set_visible(false);

        let query_entry = gtk::Entry::new();
        query_entry.set_hexpand(true);
        query_entry.set_placeholder_text(Some("Visual novel title"));
        let query_sender = sender.clone();
        query_entry.connect_changed(move |entry| {
            query_sender.input(AppMsg::QueryChanged(entry.text().to_string()));
        });
        let search_sender = sender.clone();
        query_entry.connect_activate(move |_| {
            search_sender.input(AppMsg::SearchVndb);
        });

        let results_list = gtk::ListBox::new();
        results_list.set_selection_mode(gtk::SelectionMode::None);
        results_list.add_css_class("boxed-list");
        results_list.set_visible(false);

        let model = Self {
            page: Page::Library,
            games: Vec::new(),
            library_error: None,
            query: String::new(),
            search: SearchState::Idle,
            vndb: VndbClient::new(),
            sender: sender.clone(),
            games_grid,
            query_entry,
            results_list,
        };
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, root: &Self::Root) {
        match message {
            AppMsg::ChooseSourceFile => {
                show_source_chooser(root, sender, gtk::FileChooserAction::Open);
            }
            AppMsg::AddLocal => {
                show_local_source_prompt(root, sender);
            }
            AppMsg::ChooseSourceFolder => {
                show_source_chooser(root, sender, gtk::FileChooserAction::SelectFolder);
            }
            AppMsg::SourceSelected(path) => {
                self.add_local_source(path);
            }
            AppMsg::ShowSearch => {
                self.show_search();
            }
            AppMsg::QueryChanged(query) => {
                self.query = query;
            }
            AppMsg::SearchVndb => {
                self.start_search(sender);
            }
            AppMsg::SelectMatch(entry) => {
                self.add_vndb_title(entry);
            }
            AppMsg::ShowLibrary => {
                self.show_library();
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            CommandOutput::SearchFinished { query, result } => match result {
                Ok(results) => {
                    let count = results.entries.len();
                    self.replace_search_results(results.entries);
                    self.search = SearchState::Loaded {
                        query,
                        count,
                        more: results.more,
                    };
                }
                Err(error) => {
                    self.replace_search_results(Vec::new());
                    self.search = SearchState::Failed {
                        query,
                        message: error.to_string(),
                    };
                }
            },
        }
    }
}

impl App {
    fn can_search(&self) -> bool {
        !self.query.trim().is_empty() && !self.is_searching()
    }

    fn is_searching(&self) -> bool {
        matches!(self.search, SearchState::Loading { .. })
    }

    fn search_text(&self) -> String {
        match &self.search {
            SearchState::Idle => "Enter a title. Choosing a result adds it without installing files.".to_owned(),
            SearchState::Loading { query } => format!("Searching for “{query}”…"),
            SearchState::Loaded {
                query, count: 0, ..
            } => {
                format!("No VNDB results for “{query}”.")
            }
            SearchState::Loaded { query, count, more } => {
                let noun = if *count == 1 { "result" } else { "results" };
                let suffix = if *more {
                    " More matches are available."
                } else {
                    ""
                };
                format!("{count} {noun} for “{query}”. Pick one to add.{suffix}")
            }
            SearchState::Failed { query, message } => {
                format!("Search for “{query}” failed.\n\n{message}")
            }
        }
    }

    fn add_local_source(&mut self, path: PathBuf) {
        match identify_source(path) {
            Ok(source) => {
                let title = fallback_title(&source);
                self.games.push(LibraryGame {
                    title,
                    origin: GameOrigin::Local(source),
                    thumbnail: None,
                });
                self.library_error = None;
                self.refresh_games_list();
                self.show_library();
            }
            Err(error) => {
                self.library_error = Some(error.to_string());
                self.show_library();
            }
        }
    }

    fn add_vndb_title(&mut self, entry: SearchResultWithThumbnail) {
        self.games.push(LibraryGame {
            title: entry.summary.title.clone(),
            origin: GameOrigin::Vndb(entry.summary),
            thumbnail: entry.thumbnail,
        });
        self.library_error = None;
        self.refresh_games_list();
        self.show_library();
    }

    fn show_search(&mut self) {
        self.page = Page::Search;
        self.library_error = None;
    }

    fn start_search(&mut self, sender: ComponentSender<App>) {
        if !self.can_search() {
            return;
        }

        let query = self.query.trim().to_owned();
        let client = self.vndb.clone();
        self.replace_search_results(Vec::new());
        self.search = SearchState::Loading {
            query: query.clone(),
        };
        sender.oneshot_command(async move {
            let result = search_with_thumbnails(client, &query).await;
            CommandOutput::SearchFinished { query, result }
        });
    }

    fn show_library(&mut self) {
        self.page = Page::Library;
        self.query.clear();
        self.query_entry.set_text("");
        self.search = SearchState::Idle;
        self.replace_search_results(Vec::new());
    }

    fn refresh_games_list(&self) {
        while let Some(child) = self.games_grid.first_child() {
            self.games_grid.remove(&child);
        }

        for game in &self.games {
            self.games_grid.append(&build_game_card(game));
        }
        self.games_grid
            .set_visible(self.games_grid.first_child().is_some());
    }

    fn replace_search_results(&self, entries: Vec<SearchResultWithThumbnail>) {
        while let Some(child) = self.results_list.first_child() {
            self.results_list.remove(&child);
        }

        for entry in entries {
            self.results_list
                .append(&build_result_row(entry, self.sender.clone()));
        }
        self.results_list
            .set_visible(self.results_list.first_child().is_some());
    }
}

fn show_local_source_prompt(root: &adw::ApplicationWindow, sender: ComponentSender<App>) {
    let window = gtk::Window::builder()
        .transient_for(root)
        .modal(true)
        .title("Add archive or folder")
        .resizable(false)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_all(18);

    let description = gtk::Label::new(Some(
        "The path is classified and added. Archives are not extracted here.",
    ));
    description.set_wrap(true);
    description.set_xalign(0.0);
    description.add_css_class("dim-label");
    content.append(&description);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);

    let cancel = gtk::Button::with_label("Cancel");
    let archive = gtk::Button::with_label("Archive");
    archive.add_css_class("suggested-action");
    let folder = gtk::Button::with_label("Folder");

    let cancel_window = window.clone();
    cancel.connect_clicked(move |_| cancel_window.close());

    let archive_window = window.clone();
    let archive_sender = sender.clone();
    archive.connect_clicked(move |_| {
        archive_window.close();
        archive_sender.input(AppMsg::ChooseSourceFile);
    });

    let folder_window = window.clone();
    folder.connect_clicked(move |_| {
        folder_window.close();
        sender.input(AppMsg::ChooseSourceFolder);
    });

    buttons.append(&cancel);
    buttons.append(&folder);
    buttons.append(&archive);
    content.append(&buttons);
    window.set_child(Some(&content));
    window.present();
}

fn show_source_chooser(
    root: &adw::ApplicationWindow,
    sender: ComponentSender<App>,
    action: gtk::FileChooserAction,
) {
    let title = match action {
        gtk::FileChooserAction::SelectFolder => "Choose a folder",
        _ => "Choose an archive or folder",
    };
    let chooser = gtk::FileChooserNative::new(
        Some(title),
        Some(root),
        action,
        Some("Choose"),
        Some("Cancel"),
    );

    if action == gtk::FileChooserAction::Open {
        let supported = gtk::FileFilter::new();
        supported.set_name(Some("Archives"));
        for pattern in ["*.7z", "*.rar", "*.zip"] {
            supported.add_pattern(pattern);
        }
        chooser.add_filter(&supported);

        let all_files = gtk::FileFilter::new();
        all_files.set_name(Some("All files"));
        all_files.add_pattern("*");
        chooser.add_filter(&all_files);
    }

    chooser.connect_response(move |chooser, response| {
        if response == gtk::ResponseType::Accept
            && let Some(path) = chooser.file().and_then(|file| file.path())
        {
            sender.input(AppMsg::SourceSelected(path));
        }
        chooser.destroy();
    });
    chooser.show();
}

async fn search_with_thumbnails(
    client: VndbClient,
    query: &str,
) -> Result<SearchDisplayResults, VndbError> {
    let results = client.search(query).await?;
    let downloads = results.entries.into_iter().map(|summary| {
        let client = client.clone();
        async move {
            let thumbnail = match &summary.image {
                Some(image) => client.fetch_thumbnail(&image.thumbnail).await.ok(),
                None => None,
            };
            SearchResultWithThumbnail { summary, thumbnail }
        }
    });

    Ok(SearchDisplayResults {
        entries: join_all(downloads).await,
        more: results.more,
    })
}

fn build_game_card(game: &LibraryGame) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("card");
    card.add_css_class("library-card");
    card.set_hexpand(false);
    card.set_halign(gtk::Align::Center);

    card.append(&cover_widget(
        game.thumbnail.clone(),
        &format!("Cover for {}", game.title),
        CARD_COVER_WIDTH,
        CARD_COVER_HEIGHT,
        "library-cover",
    ));

    let title = gtk::Label::new(Some(&game.title));
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_justify(gtk::Justification::Center);
    title.set_xalign(0.5);
    title.set_max_width_chars(18);
    title.set_lines(2);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.add_css_class("heading");
    card.append(&title);

    let subtitle = gtk::Label::new(Some(&game_subtitle(game)));
    subtitle.set_wrap(true);
    subtitle.set_justify(gtk::Justification::Center);
    subtitle.set_xalign(0.5);
    subtitle.set_max_width_chars(18);
    subtitle.set_lines(2);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
    subtitle.add_css_class("dim-label");
    subtitle.add_css_class("caption");
    card.append(&subtitle);

    card
}

fn game_subtitle(game: &LibraryGame) -> String {
    match &game.origin {
        GameOrigin::Local(source) => format!("{}\n{}", source.kind, source.path.display()),
        GameOrigin::Vndb(summary) => {
            let mut subtitle = summary.id.clone();
            if let Some(released) = &summary.released {
                subtitle.push_str(" · ");
                subtitle.push_str(released);
            }
            subtitle.push_str("\nNo local files");
            subtitle
        }
    }
}

fn build_result_row(
    entry: SearchResultWithThumbnail,
    sender: ComponentSender<App>,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&entry.summary.title);
    row.set_subtitle(&result_subtitle(&entry.summary));
    row.set_activatable(true);
    row.add_prefix(&cover_widget(
        entry.thumbnail.clone(),
        &format!("Cover for {}", entry.summary.title),
        COVER_WIDTH,
        COVER_HEIGHT,
        "vndb-cover",
    ));
    row.connect_activated(move |_| {
        sender.input(AppMsg::SelectMatch(entry.clone()));
    });
    row
}

fn cover_widget(
    thumbnail: Option<Vec<u8>>,
    alternative_text: &str,
    width: i32,
    height: i32,
    css_class: &str,
) -> gtk::Widget {
    thumbnail
        .and_then(|bytes| thumbnail_texture(bytes, width, height))
        .map(|texture| {
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_alternative_text(Some(alternative_text));
            picture.set_can_shrink(true);
            apply_cover_slot(&picture, width, height, css_class);
            picture.upcast()
        })
        .unwrap_or_else(|| {
            let image = gtk::Image::from_icon_name("image-missing-symbolic");
            image.set_pixel_size(32);
            image.add_css_class("dim-label");
            apply_cover_slot(&image, width, height, css_class);
            image.upcast()
        })
}

fn thumbnail_texture(thumbnail: Vec<u8>, width: i32, height: i32) -> Option<gtk::gdk::Texture> {
    let stream = gtk::gio::MemoryInputStream::from_bytes(&gtk::glib::Bytes::from_owned(thumbnail));
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_stream(&stream, gtk::gio::Cancellable::NONE).ok()?;
    let (x, y, crop_width, crop_height) = cover_crop_region(pixbuf.width(), pixbuf.height(), 2, 3);
    let cropped = pixbuf.new_subpixbuf(x, y, crop_width, crop_height);
    let scaled = cropped.scale_simple(width, height, gtk::gdk_pixbuf::InterpType::Bilinear)?;
    Some(gtk::gdk::Texture::for_pixbuf(&scaled))
}

fn apply_cover_slot(widget: &impl gtk::prelude::WidgetExt, width: i32, height: i32, css_class: &str) {
    widget.set_size_request(width, height);
    widget.set_hexpand(false);
    widget.set_vexpand(false);
    widget.set_halign(gtk::Align::Center);
    widget.set_valign(gtk::Align::Center);
    widget.add_css_class(css_class);
}

fn cover_crop_region(
    width: i32,
    height: i32,
    ratio_width: i32,
    ratio_height: i32,
) -> (i32, i32, i32, i32) {
    if width <= 0 || height <= 0 || ratio_width <= 0 || ratio_height <= 0 {
        return (0, 0, width.max(0), height.max(0));
    }

    if width * ratio_height >= height * ratio_width {
        let crop_width =
            (i64::from(height) * i64::from(ratio_width) / i64::from(ratio_height)) as i32;
        let crop_width = crop_width.clamp(1, width);
        ((width - crop_width) / 2, 0, crop_width, height)
    } else {
        let crop_height =
            (i64::from(width) * i64::from(ratio_height) / i64::from(ratio_width)) as i32;
        let crop_height = crop_height.clamp(1, height);
        (0, (height - crop_height) / 2, width, crop_height)
    }
}

fn result_subtitle(entry: &VnSummary) -> String {
    let mut subtitle = String::new();
    if let Some(alternative) = &entry.alttitle
        && alternative != &entry.title
    {
        subtitle.push_str(alternative);
        subtitle.push('\n');
    }
    subtitle.push_str(&entry.id);
    if let Some(released) = &entry.released {
        subtitle.push_str(" · ");
        subtitle.push_str(released);
    }
    subtitle
}

fn guess_search_query(path: &Path) -> String {
    let raw = path
        .file_stem()
        .or_else(|| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    clean_title_guess(raw)
}

fn fallback_title(source: &IdentifiedSource) -> String {
    let guess = guess_search_query(&source.path);
    if guess.is_empty() {
        source
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled")
            .to_owned()
    } else {
        guess
    }
}

fn clean_title_guess(raw: &str) -> String {
    let stripped = strip_grouped_tags(raw);
    let mut text = stripped.replace('_', " ");
    if !text.contains(' ') && text.chars().filter(|character| *character == '.').count() >= 1 {
        text = text.replace('.', " ");
    }
    collapse_whitespace(&text)
        .trim_matches(|character: char| character.is_ascii_punctuation())
        .to_owned()
}

fn strip_grouped_tags(raw: &str) -> String {
    let mut out = String::new();
    let mut square = 0_u32;
    let mut paren = 0_u32;
    for character in raw.chars() {
        match character {
            '[' => square = square.saturating_add(1),
            ']' => square = square.saturating_sub(1),
            '(' => paren = paren.saturating_add(1),
            ')' => paren = paren.saturating_sub(1),
            _ if square == 0 && paren == 0 => out.push(character),
            _ => {}
        }
    }
    out
}

fn collapse_whitespace(raw: &str) -> String {
    let mut out = String::new();
    let mut previous_space = false;
    for character in raw.chars() {
        if character.is_whitespace() {
            if !previous_space && !out.is_empty() {
                out.push(' ');
            }
            previous_space = true;
        } else {
            out.push(character);
            previous_space = false;
        }
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        stages::identify::{IdentifiedSource, SourceKind},
        vndb::VnSummary,
    };
    use relm4::gtk::{self, prelude::*};

    use super::{GameOrigin, LibraryGame, result_subtitle};

    #[test]
    fn formats_original_title_and_release_for_result_rows() {
        let entry = VnSummary {
            id: "v1".to_owned(),
            title: "Primary".to_owned(),
            alttitle: Some("原題".to_owned()),
            released: Some("2025".to_owned()),
            image: None,
        };

        assert_eq!(result_subtitle(&entry), "原題\nv1 · 2025");
    }

    #[test]
    fn cover_crop_selects_centered_2_by_3_region() {
        assert_eq!(super::cover_crop_region(200, 200, 2, 3), (33, 0, 133, 200));
        assert_eq!(super::cover_crop_region(400, 200, 2, 3), (133, 0, 133, 200));
        assert_eq!(super::cover_crop_region(200, 400, 2, 3), (0, 50, 200, 300));
        assert_eq!(super::cover_crop_region(200, 300, 2, 3), (0, 0, 200, 300));
    }

    #[test]
    fn thumbnail_texture_normalizes_to_cover_slot() {
        gtk::init().expect("gtk init");
        let image = gtk::Image::from_icon_name("image-missing-symbolic");
        super::apply_cover_slot(&image, super::COVER_WIDTH, super::COVER_HEIGHT, "vndb-cover");
        assert_eq!(image.width_request(), super::COVER_WIDTH);
        assert_eq!(image.height_request(), super::COVER_HEIGHT);
        assert!(image.has_css_class("vndb-cover"));

        let pixbuf = gtk::gdk_pixbuf::Pixbuf::new(
            gtk::gdk_pixbuf::Colorspace::Rgb,
            false,
            8,
            400,
            200,
        )
        .expect("pixbuf");
        pixbuf.fill(0x3366_99ff);
        let png = pixbuf.save_to_bufferv("png", &[]).expect("encode png");
        let texture = super::thumbnail_texture(png, super::COVER_WIDTH, super::COVER_HEIGHT)
            .expect("decode cover");
        assert_eq!(texture.width(), super::COVER_WIDTH);
        assert_eq!(texture.height(), super::COVER_HEIGHT);

        let card = super::build_game_card(&super::LibraryGame {
            title: "Primary".to_owned(),
            origin: super::GameOrigin::Vndb(VnSummary {
                id: "v1".to_owned(),
                title: "Primary".to_owned(),
                alttitle: None,
                released: None,
                image: None,
            }),
            thumbnail: None,
        });
        assert!(card.has_css_class("library-card"));
        assert!(card.has_css_class("card"));
    }

    #[test]
    fn local_add_uses_cleaned_file_name_without_vndb() {
        let source = IdentifiedSource {
            path: PathBuf::from("[Group] Subarashiki_Hibi [ENG].7z"),
            kind: SourceKind::Archive(crate::stages::identify::ArchiveFormat::SevenZip),
        };
        assert_eq!(super::fallback_title(&source), "Subarashiki Hibi");
        assert_eq!(
            super::game_subtitle(&LibraryGame {
                title: "Subarashiki Hibi".to_owned(),
                origin: GameOrigin::Local(source),
                thumbnail: None,
            }),
            "7-Zip archive\n[Group] Subarashiki_Hibi [ENG].7z"
        );
    }

    #[test]
    fn vndb_add_records_metadata_without_local_files() {
        let game = LibraryGame {
            title: "Primary".to_owned(),
            origin: GameOrigin::Vndb(VnSummary {
                id: "v1".to_owned(),
                title: "Primary".to_owned(),
                alttitle: None,
                released: Some("2010-03-26".to_owned()),
                image: None,
            }),
            thumbnail: None,
        };

        assert_eq!(super::game_subtitle(&game), "v1 · 2010-03-26\nNo local files");
    }

    #[test]
    fn guess_search_query_strips_release_tags() {
        assert_eq!(
            super::guess_search_query(PathBuf::from("[Group] Subarashiki_Hibi [ENG].7z").as_path()),
            "Subarashiki Hibi"
        );
        assert_eq!(
            super::guess_search_query(PathBuf::from("Wonderful.Everyday.iso").as_path()),
            "Wonderful Everyday"
        );
        assert_eq!(
            super::guess_search_query(PathBuf::from("[ENG].zip").as_path()),
            ""
        );
        assert_eq!(
            super::guess_search_query(PathBuf::from("素晴らしき日々.7z").as_path()),
            "素晴らしき日々"
        );
    }
}
