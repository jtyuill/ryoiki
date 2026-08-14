use futures_util::future::join_all;

use std::path::{Path, PathBuf};

use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, adw, adw::prelude::*, gtk};

use crate::{
    stages::identify::{IdentifiedSource, identify_source},
    vndb::{VnSummary, VndbClient, VndbError},
};

const COVER_WIDTH: i32 = 72;
const COVER_HEIGHT: i32 = 108;

pub struct App {
    page: Page,
    games: Vec<LibraryGame>,
    source: Option<IdentifiedSource>,
    source_error: Option<String>,
    query: String,
    search: SearchState,
    vndb: VndbClient,
    sender: ComponentSender<App>,
    games_list: gtk::ListBox,
    query_entry: gtk::Entry,
    results_list: gtk::ListBox,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Library,
    Identify,
}

impl Page {
    fn name(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Identify => "identify",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Library => "Library",
            Self::Identify => "Add game",
        }
    }
}

#[derive(Clone, Debug)]
struct LibraryGame {
    title: String,
    source: IdentifiedSource,
    thumbnail: Option<Vec<u8>>,
}

#[derive(Debug)]
pub enum AppMsg {
    ChooseSourceFile,
    ChooseSourceFolder,
    SourceSelected(PathBuf),
    QueryChanged(String),
    SearchVndb,
    SelectMatch(SearchResultWithThumbnail),
    AddWithoutMetadata,
    ShowLibrary,
}

#[derive(Debug)]
pub enum CommandOutput {
    SearchFinished {
        query: String,
        auto: bool,
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
                        set_visible: model.page == Page::Identify,
                        connect_clicked => AppMsg::ShowLibrary,
                    },

                    #[wrap(Some)]
                    set_title_widget = &gtk::Label {
                        #[watch]
                        set_label: model.page.title(),
                        add_css_class: "heading",
                    },

                    pack_end = &gtk::MenuButton {
                        set_label: "Add game",
                        set_icon_name: "list-add-symbolic",
                        set_always_show_arrow: true,
                        #[watch]
                        set_visible: model.page == Page::Library,

                        #[wrap(Some)]
                        set_popover = &gtk::Popover {
                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_spacing: 6,
                                set_margin_all: 6,

                                gtk::Button {
                                    set_label: "Add archive",
                                    add_css_class: "flat",
                                    connect_clicked => AppMsg::ChooseSourceFile,
                                },

                                gtk::Button {
                                    set_label: "Add folder",
                                    add_css_class: "flat",
                                    connect_clicked => AppMsg::ChooseSourceFolder,
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
            add_named: (&identify_page, Some("identify")),
            #[watch]
            set_visible_child_name: model.page.name(),
        },

        library_page = &gtk::ScrolledWindow {
            set_hscrollbar_policy: gtk::PolicyType::Never,

            #[wrap(Some)]
            set_child = &adw::Clamp {
                set_hexpand: true,
                set_maximum_size: 720,
                set_tightening_threshold: 520,

                #[wrap(Some)]
                set_child = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 24,
                    set_margin_all: 32,

                    adw::StatusPage {
                        set_icon_name: Some("folder-documents-symbolic"),
                        set_title: "No games yet",
                        set_description: Some("Add an archive. ryoiki will try the file name, then VNDB."),
                        #[watch]
                        set_visible: model.games.is_empty(),
                    },

                    append: &model.games_list,
                },
            },
        },

        identify_page = &gtk::ScrolledWindow {
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
                        set_label: "Identify this source",
                        set_xalign: 0.0,
                        add_css_class: "title-1",
                    },

                    gtk::Label {
                        set_label: "The file name is searched first. If that is not enough, pick a VNDB title.",
                        set_xalign: 0.0,
                        set_wrap: true,
                        add_css_class: "dim-label",
                    },

                    adw::PreferencesGroup {
                        set_title: "Source",
                        set_description: Some("User-owned files only"),

                        adw::ActionRow {
                            set_title: "Selected source",
                            #[watch]
                            set_subtitle: &model.source_description(),
                        },
                    },

                    gtk::Label {
                        #[watch]
                        set_label: &model.source_status(),
                        #[watch]
                        set_visible: model.has_source_status(),
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_selectable: true,
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

                    gtk::Button {
                        set_label: "Add without metadata",
                        #[watch]
                        set_sensitive: model.source.is_some(),
                        connect_clicked => AppMsg::AddWithoutMetadata,
                    },
                },
            },
        },
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let games_list = gtk::ListBox::new();
        games_list.set_selection_mode(gtk::SelectionMode::None);
        games_list.add_css_class("boxed-list");
        games_list.set_visible(false);

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
            source: None,
            source_error: None,
            query: String::new(),
            search: SearchState::Idle,
            vndb: VndbClient::new(),
            sender: sender.clone(),
            games_list,
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
            AppMsg::ChooseSourceFolder => {
                show_source_chooser(root, sender, gtk::FileChooserAction::SelectFolder);
            }
            AppMsg::SourceSelected(path) => {
                self.open_source(path, sender);
            }
            AppMsg::QueryChanged(query) => {
                self.query = query;
            }
            AppMsg::SearchVndb => {
                self.start_search(sender, false);
            }
            AppMsg::SelectMatch(entry) => {
                self.add_game(Some(entry));
            }
            AppMsg::AddWithoutMetadata => {
                self.add_game(None);
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
            CommandOutput::SearchFinished {
                query,
                auto,
                result,
            } => match result {
                Ok(results) if auto && should_accept_auto_match(&results) => {
                    let entry = results
                        .entries
                        .into_iter()
                        .next()
                        .expect("auto-match requires one result");
                    self.add_game(Some(entry));
                }
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
    fn source_description(&self) -> String {
        self.source
            .as_ref()
            .map(|source| source.path.display().to_string())
            .unwrap_or_else(|| "Nothing selected".to_owned())
    }

    fn source_status(&self) -> String {
        if let Some(error) = &self.source_error {
            return error.clone();
        }

        self.source
            .as_ref()
            .map(|source| format!("Detected: {}", source.kind))
            .unwrap_or_default()
    }

    fn has_source_status(&self) -> bool {
        self.source.is_some() || self.source_error.is_some()
    }

    fn can_search(&self) -> bool {
        !self.query.trim().is_empty() && !self.is_searching()
    }

    fn is_searching(&self) -> bool {
        matches!(self.search, SearchState::Loading { .. })
    }

    fn search_text(&self) -> String {
        match &self.search {
            SearchState::Idle => {
                "A unique file-name match is added automatically. Otherwise search VNDB."
                    .to_owned()
            }
            SearchState::Loading { query } => format!("Searching for “{query}”…"),
            SearchState::Loaded {
                query, count: 0, ..
            } => {
                format!("No VNDB results for “{query}”. Search again or add without metadata.")
            }
            SearchState::Loaded { query, count, more } => {
                let noun = if *count == 1 { "result" } else { "results" };
                let suffix = if *more {
                    " More matches are available."
                } else {
                    ""
                };
                format!("{count} {noun} for “{query}”. Pick one.{suffix}")
            }
            SearchState::Failed { query, message } => {
                format!("Search for “{query}” failed.\n\n{message}")
            }
        }
    }

    fn open_source(&mut self, path: PathBuf, sender: ComponentSender<App>) {
        self.page = Page::Identify;
        self.replace_search_results(Vec::new());
        match identify_source(path) {
            Ok(source) => {
                let query = guess_search_query(&source.path);
                self.source = Some(source);
                self.source_error = None;
                self.query = query.clone();
                self.query_entry.set_text(&query);
                if query.is_empty() {
                    self.search = SearchState::Idle;
                } else {
                    self.start_search(sender, true);
                }
            }
            Err(error) => {
                self.source = None;
                self.source_error = Some(error.to_string());
                self.query.clear();
                self.query_entry.set_text("");
                self.search = SearchState::Idle;
            }
        }
    }

    fn start_search(&mut self, sender: ComponentSender<App>, auto: bool) {
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
            CommandOutput::SearchFinished {
                query,
                auto,
                result,
            }
        });
    }

    fn add_game(&mut self, matched: Option<SearchResultWithThumbnail>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let (title, thumbnail) = match matched {
            Some(entry) => (entry.summary.title, entry.thumbnail),
            None => (fallback_title(&source), None),
        };
        self.games.push(LibraryGame {
            title,
            source,
            thumbnail,
        });
        self.refresh_games_list();
        self.show_library();
    }

    fn show_library(&mut self) {
        self.page = Page::Library;
        self.source = None;
        self.source_error = None;
        self.query.clear();
        self.query_entry.set_text("");
        self.search = SearchState::Idle;
        self.replace_search_results(Vec::new());
    }

    fn refresh_games_list(&self) {
        while let Some(child) = self.games_list.first_child() {
            self.games_list.remove(&child);
        }

        for game in &self.games {
            self.games_list.append(&build_game_row(game));
        }
        self.games_list
            .set_visible(self.games_list.first_child().is_some());
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

fn show_source_chooser(
    root: &adw::ApplicationWindow,
    sender: ComponentSender<App>,
    action: gtk::FileChooserAction,
) {
    let title = match action {
        gtk::FileChooserAction::SelectFolder => "Choose a source folder",
        _ => "Choose an archive, disc image, or installer",
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
        supported.set_name(Some("Supported sources"));
        for pattern in ["*.7z", "*.rar", "*.zip", "*.iso", "*.mds", "*.exe"] {
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

fn build_game_row(game: &LibraryGame) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&game.title);
    row.set_subtitle(&format!(
        "{}\n{}",
        game.source.kind,
        game.source.path.display()
    ));
    row.add_prefix(&cover_widget(
        game.thumbnail.clone(),
        &format!("Cover for {}", game.title),
    ));
    row
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
    ));
    row.connect_activated(move |_| {
        sender.input(AppMsg::SelectMatch(entry.clone()));
    });
    row
}

fn cover_widget(thumbnail: Option<Vec<u8>>, alternative_text: &str) -> gtk::Widget {
    thumbnail
        .and_then(thumbnail_texture)
        .map(|texture| {
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_alternative_text(Some(alternative_text));
            picture.set_can_shrink(true);
            apply_cover_slot(&picture);
            picture.upcast()
        })
        .unwrap_or_else(|| {
            let image = gtk::Image::from_icon_name("image-missing-symbolic");
            image.set_pixel_size(32);
            image.add_css_class("dim-label");
            apply_cover_slot(&image);
            image.upcast()
        })
}

fn thumbnail_texture(thumbnail: Vec<u8>) -> Option<gtk::gdk::Texture> {
    let stream = gtk::gio::MemoryInputStream::from_bytes(&gtk::glib::Bytes::from_owned(thumbnail));
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_stream(&stream, gtk::gio::Cancellable::NONE).ok()?;
    let (x, y, width, height) = cover_crop_region(pixbuf.width(), pixbuf.height(), 2, 3);
    let cropped = pixbuf.new_subpixbuf(x, y, width, height);
    let scaled = cropped.scale_simple(
        COVER_WIDTH,
        COVER_HEIGHT,
        gtk::gdk_pixbuf::InterpType::Bilinear,
    )?;
    Some(gtk::gdk::Texture::for_pixbuf(&scaled))
}

fn apply_cover_slot(widget: &impl gtk::prelude::WidgetExt) {
    widget.set_size_request(COVER_WIDTH, COVER_HEIGHT);
    widget.set_hexpand(false);
    widget.set_vexpand(false);
    widget.set_halign(gtk::Align::Center);
    widget.set_valign(gtk::Align::Center);
    widget.add_css_class("vndb-cover");
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

fn should_accept_auto_match(results: &SearchDisplayResults) -> bool {
    results.entries.len() == 1 && !results.more
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::vndb::VnSummary;
    use relm4::gtk::{self, prelude::*};

    use super::result_subtitle;

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
        super::apply_cover_slot(&image);
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
        let texture = super::thumbnail_texture(png).expect("decode cover");
        assert_eq!(texture.width(), super::COVER_WIDTH);
        assert_eq!(texture.height(), super::COVER_HEIGHT);
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

    #[test]
    fn unique_auto_match_is_accepted_only_when_unambiguous() {
        let unique = super::SearchDisplayResults {
            entries: vec![super::SearchResultWithThumbnail {
                summary: VnSummary {
                    id: "v1".to_owned(),
                    title: "One".to_owned(),
                    alttitle: None,
                    released: None,
                    image: None,
                },
                thumbnail: None,
            }],
            more: false,
        };
        let ambiguous = super::SearchDisplayResults {
            entries: unique.entries.clone(),
            more: true,
        };
        let empty = super::SearchDisplayResults {
            entries: Vec::new(),
            more: false,
        };

        assert!(super::should_accept_auto_match(&unique));
        assert!(!super::should_accept_auto_match(&ambiguous));
        assert!(!super::should_accept_auto_match(&empty));
    }
}
