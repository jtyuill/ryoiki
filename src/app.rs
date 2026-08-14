use futures_util::future::join_all;

use std::path::PathBuf;

use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, adw, adw::prelude::*, gtk};

use crate::{
    stages::identify::{IdentifiedSource, identify_source},
    vndb::{VnSummary, VndbClient, VndbError},
};

pub struct App {
    source: Option<IdentifiedSource>,
    source_error: Option<String>,
    query: String,
    search: SearchState,
    vndb: VndbClient,
    results_list: gtk::ListBox,
}

#[derive(Debug)]
pub enum AppMsg {
    ChooseSourceFile,
    ChooseSourceFolder,
    SourceSelected(PathBuf),
    QueryChanged(String),
    SearchVndb,
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

#[derive(Debug)]
struct SearchResultWithThumbnail {
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
                    #[wrap(Some)]
                    set_title_widget = &gtk::StackSwitcher {
                        set_stack: Some(&page_stack),
                    },
                },

                append: &page_stack,
            },
        },

        page_stack = &gtk::Stack {
            set_vexpand: true,
            set_transition_type: gtk::StackTransitionType::Crossfade,
            set_transition_duration: 150,
            set_vhomogeneous: false,
            add_titled: (&setup_page, Some("setup"), "Setup"),
            add_titled: (&metadata_page, Some("metadata"), "VNDB"),
        },

        setup_page = &gtk::ScrolledWindow {
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

                    gtk::Label {
                        set_label: "Set up a visual novel",
                        set_xalign: 0.0,
                        add_css_class: "title-1",
                    },

                    gtk::Label {
                        set_label: "Choose an archive, disc image, installer, or folder. ryoiki will inspect it before changing a Wine prefix.",
                        set_xalign: 0.0,
                        set_wrap: true,
                        add_css_class: "dim-label",
                    },

                    adw::PreferencesGroup {
                        set_title: "Installation source",
                        set_description: Some("User-owned files only"),

                        adw::ActionRow {
                            set_title: "Selected source",
                            #[watch]
                            set_subtitle: &model.source_description(),

                            add_suffix = &gtk::Box {
                                set_spacing: 6,
                                set_valign: gtk::Align::Center,

                                gtk::Button {
                                    set_label: "Choose file",
                                    connect_clicked => AppMsg::ChooseSourceFile,
                                },

                                gtk::Button {
                                    set_label: "Choose folder",
                                    connect_clicked => AppMsg::ChooseSourceFolder,
                                },
                            },
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
                },
            },
        },

        metadata_page = &gtk::ScrolledWindow {
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
                        set_label: "Search VNDB",
                        set_xalign: 0.0,
                        add_css_class: "title-1",
                    },

                    gtk::Label {
                        set_label: "Search the VNDB Kana API and inspect the metadata that can later be attached to a launch profile.",
                        set_xalign: 0.0,
                        set_wrap: true,
                        add_css_class: "dim-label",
                    },

                    gtk::Box {
                        set_spacing: 8,

                        gtk::Entry {
                            set_hexpand: true,
                            set_placeholder_text: Some("Visual novel title"),
                            connect_changed[sender] => move |entry| {
                                sender.input(AppMsg::QueryChanged(entry.text().to_string()));
                            },
                            connect_activate => AppMsg::SearchVndb,
                        },

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
        let results_list = gtk::ListBox::new();
        results_list.set_selection_mode(gtk::SelectionMode::None);
        results_list.add_css_class("boxed-list");
        results_list.set_visible(false);

        let model = Self {
            source: None,
            source_error: None,
            query: String::new(),
            search: SearchState::Idle,
            vndb: VndbClient::new(),
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
            AppMsg::SourceSelected(path) => match identify_source(path) {
                Ok(source) => {
                    self.source = Some(source);
                    self.source_error = None;
                }
                Err(error) => {
                    self.source = None;
                    self.source_error = Some(error.to_string());
                }
            },
            AppMsg::QueryChanged(query) => {
                self.query = query;
            }
            AppMsg::SearchVndb => {
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
            SearchState::Idle => "Enter a title to query VNDB.".to_owned(),
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
                format!("{count} {noun} for “{query}”.{suffix}")
            }
            SearchState::Failed { query, message } => {
                format!("Search for “{query}” failed.\n\n{message}")
            }
        }
    }

    fn replace_search_results(&self, entries: Vec<SearchResultWithThumbnail>) {
        while let Some(child) = self.results_list.first_child() {
            self.results_list.remove(&child);
        }

        for entry in entries {
            self.results_list.append(&build_result_row(entry));
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

fn build_result_row(entry: SearchResultWithThumbnail) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&entry.summary.title);
    row.set_subtitle(&result_subtitle(&entry.summary));
    let alternative_text = format!("Cover for {}", entry.summary.title);

    let cover: gtk::Widget = entry
        .thumbnail
        .and_then(|thumbnail| {
            let bytes = gtk::glib::Bytes::from_owned(thumbnail);
            gtk::gdk::Texture::from_bytes(&bytes).ok()
        })
        .map(|texture| {
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_alternative_text(Some(&alternative_text));
            picture.set_can_shrink(true);
            picture.set_size_request(72, 100);
            picture.upcast()
        })
        .unwrap_or_else(|| {
            let image = gtk::Image::from_icon_name("image-missing-symbolic");
            image.set_pixel_size(32);
            image.set_size_request(72, 100);
            image.add_css_class("dim-label");
            image.upcast()
        });
    row.add_prefix(&cover);
    row
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

#[cfg(test)]
mod tests {
    use crate::vndb::VnSummary;

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
}
