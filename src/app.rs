use futures_util::future::join_all;

use std::{
    fs,
    path::{Path, PathBuf},
    thread,
};

use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, adw, adw::prelude::*, gtk};

use crate::{
    profile::{
        GameMetadata, LibraryError, LibraryStore, PrefixArch, StoredGame, StoredProfile, data_root,
    },
    stages::{
        identify::{IdentifiedSource, SourceKind, identify_source},
        install::{
            InstallError, InstallOutcome, InstallRequest, PreparedInstall, PreparedInstallSource,
            RuntimeCommands, execute_install, inspect_install_source, prepare_install_source,
        },
        launch::{LaunchError, run_game},
    },
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
    library_edit_mode: bool,
    query: String,
    search: SearchState,
    vndb: VndbClient,
    sender: ComponentSender<App>,
    games_grid: gtk::FlowBox,
    query_entry: gtk::Entry,
    results_list: gtk::ListBox,
    wizard: Option<InstallWizard>,
    library_store: Option<LibraryStore>,
    data_root: Option<PathBuf>,
    wizard_title_entry: gtk::Entry,
    wizard_arch_dropdown: gtk::DropDown,
    wizard_disc_dropdown: gtk::DropDown,
    wizard_installer_dropdown: gtk::DropDown,
    wizard_executable_dropdown: gtk::DropDown,
    search_target: Option<LibraryGameId>,
    install_target: Option<LibraryGameId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Library,
    Search,
    Install,
}

impl Page {
    fn name(self) -> &'static str {
        match self {
            Self::Library => "library",
            Self::Search => "search",
            Self::Install => "install",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Library => "Library",
            Self::Search => "Search VNDB",
            Self::Install => "Install game",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LibraryGameId(pub i64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoveDirection {
    Previous,
    Next,
}

#[derive(Clone, Debug)]
struct LibraryGame {
    id: LibraryGameId,
    metadata: GameMetadata,
    profile: Option<StoredProfile>,
    thumbnail: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WizardPhase {
    Review,
    Preparing,
    ChooseDisc,
    Inspecting,
    ChooseInstaller,
    Installing,
    ChooseExecutable,
    Saving,
}

#[derive(Clone, Debug)]
struct InstallWizard {
    source: IdentifiedSource,
    title: String,
    phase: WizardPhase,
    prepared_source: Option<PreparedInstallSource>,
    prepared: Option<PreparedInstall>,
    outcome: Option<InstallOutcome>,
    selected_executable: Option<PathBuf>,
    error: Option<String>,
}

#[derive(Debug)]
pub enum AppMsg {
    ChooseSourceFile,
    ChooseSourceFolder,
    SourceSelected(PathBuf),
    ChooseGameFiles(LibraryGameId),
    WizardTitleChanged(String),
    WizardPrimary,
    WizardBrowseExecutable,
    WizardExecutableSelected(PathBuf),
    SourcePrepared(Result<PreparedInstallSource, InstallError>),
    SourceInspected {
        original: PreparedInstallSource,
        result: Result<PreparedInstall, InstallError>,
    },
    InstallationFinished {
        prepared: PreparedInstall,
        result: Result<InstallOutcome, InstallError>,
    },
    ProfileSaved {
        outcome: InstallOutcome,
        result: Result<StoredProfile, LibraryError>,
    },
    LaunchGame(LibraryGameId),
    GameFinished {
        title: String,
        result: Result<(), LaunchError>,
    },
    ToggleLibraryEditMode,
    RequestRemoveGame(LibraryGameId),
    ConfirmRemoveGame(LibraryGameId),
    ConfirmDeleteGameFiles(LibraryGameId),
    GameRemovalFinished {
        id: LibraryGameId,
        result: Result<Option<String>, String>,
    },
    MoveGame {
        id: LibraryGameId,
        direction: MoveDirection,
    },
    AddVndbOnly(SearchResultWithThumbnail),
    AddVndbAndFiles(SearchResultWithThumbnail),
    EditMetadata(LibraryGameId),
    SearchMetadata(LibraryGameId),
    SaveManualMetadata {
        id: LibraryGameId,
        metadata: GameMetadata,
    },
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
                        #[watch]
                        set_tooltip_text: Some(model.back_tooltip()),
                        #[watch]
                        set_visible: model.page != Page::Library,
                        #[watch]
                        set_sensitive: model.can_leave_page(),
                        connect_clicked => AppMsg::ShowLibrary,
                    },

                    #[wrap(Some)]
                    set_title_widget = &gtk::Label {
                        #[watch]
                        set_label: model.page.title(),
                        add_css_class: "heading",
                    },

                    pack_end = &gtk::Button {
                        #[watch]
                        set_label: if model.library_edit_mode { "Done" } else { "Edit" },
                        #[watch]
                        set_visible: model.page == Page::Library && !model.games.is_empty(),
                        connect_clicked => AppMsg::ToggleLibraryEditMode,
                    },

                    pack_end = &gtk::MenuButton {
                        set_icon_name: "list-add-symbolic",
                        set_tooltip_text: Some("Add game"),
                        #[watch]
                        set_visible: model.page == Page::Library && !model.library_edit_mode,

                        #[wrap(Some)]
                        set_popover = &gtk::Popover {
                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_spacing: 6,
                                set_margin_all: 6,


                                gtk::Button {
                                    set_label: "Add from VNDB",
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
            add_named: (&install_page, Some("install")),
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
                set_vexpand: false,
                set_valign: gtk::Align::Start,

                adw::StatusPage {
                    set_icon_name: Some("folder-documents-symbolic"),
                    set_title: "No games yet",
                    set_description: Some("Press + to add a VNDB title to the library."),
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
                        #[watch]
                        set_label: model.search_heading(),
                        set_xalign: 0.0,
                        add_css_class: "title-1",
                    },

                    gtk::Label {
                        #[watch]
                        set_label: model.search_description(),
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

        install_page = &gtk::ScrolledWindow {
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
                        set_label: "Install a visual novel",
                        set_xalign: 0.0,
                        add_css_class: "title-1",
                    },

                    gtk::Label {
                        #[watch]
                        set_label: &model.wizard_status_text(),
                        set_xalign: 0.0,
                        set_wrap: true,
                        add_css_class: "dim-label",
                    },

                    gtk::Separator {},

                    gtk::Label {
                        set_label: "Source",
                        set_xalign: 0.0,
                        add_css_class: "heading",
                    },

                    gtk::Label {
                        #[watch]
                        set_label: &model.wizard_source_text(),
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_selectable: true,
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 8,
                        #[watch]
                        set_visible: model.wizard_phase_is(WizardPhase::Review),

                        gtk::Label {
                            set_label: "Library title",
                            set_xalign: 0.0,
                            add_css_class: "heading",
                        },

                        append: &model.wizard_title_entry,

                        gtk::Label {
                            set_label: "Wine prefix",
                            set_xalign: 0.0,
                            add_css_class: "heading",
                        },

                        append: &model.wizard_arch_dropdown,

                        gtk::Label {
                            set_label: "The WoW64 prefix runs both 32-bit and 64-bit games. A private ja_JP.UTF-8 locale is generated when the system does not provide one.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            add_css_class: "dim-label",
                        },
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 8,
                        #[watch]
                        set_visible: model.wizard_phase_is(WizardPhase::ChooseDisc),

                        gtk::Label {
                            set_label: "Choose disc",
                            set_xalign: 0.0,
                            add_css_class: "heading",
                        },

                        append: &model.wizard_disc_dropdown,
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 8,
                        #[watch]
                        set_visible: model.wizard_phase_is(WizardPhase::ChooseInstaller),

                        gtk::Label {
                            set_label: "Choose installer",
                            set_xalign: 0.0,
                            add_css_class: "heading",
                        },

                        append: &model.wizard_installer_dropdown,
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 8,
                        #[watch]
                        set_visible: model.wizard_phase_is(WizardPhase::ChooseExecutable),

                        gtk::Label {
                            set_label: "Installed program",
                            set_xalign: 0.0,
                            add_css_class: "heading",
                        },

                        append: &model.wizard_executable_dropdown,

                        gtk::Button {
                            set_label: "Choose another executable…",
                            set_halign: gtk::Align::Start,
                            connect_clicked => AppMsg::WizardBrowseExecutable,
                        },

                        gtk::Label {
                            #[watch]
                            set_label: &model.wizard_selected_executable_text(),
                            #[watch]
                            set_visible: model.wizard_has_manual_executable(),
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_selectable: true,
                            add_css_class: "dim-label",
                        },
                    },

                    gtk::Label {
                        #[watch]
                        set_label: &model.wizard_error_text(),
                        #[watch]
                        set_visible: model.wizard_has_error(),
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_selectable: true,
                        add_css_class: "error",
                    },

                    gtk::Spinner {
                        #[watch]
                        set_spinning: model.wizard_is_busy(),
                        #[watch]
                        set_visible: model.wizard_is_busy(),
                        set_halign: gtk::Align::Start,
                    },

                    gtk::Button {
                        #[watch]
                        set_label: model.wizard_primary_label(),
                        #[watch]
                        set_sensitive: model.wizard_can_continue(),
                        set_halign: gtk::Align::End,
                        add_css_class: "suggested-action",
                        connect_clicked => AppMsg::WizardPrimary,
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
        let games_grid = gtk::FlowBox::new();
        games_grid.set_selection_mode(gtk::SelectionMode::None);
        games_grid.set_homogeneous(false);
        games_grid.set_min_children_per_line(1);
        games_grid.set_max_children_per_line(8);
        games_grid.set_column_spacing(12);
        games_grid.set_row_spacing(12);
        games_grid.set_valign(gtk::Align::Start);
        games_grid.set_hexpand(true);
        games_grid.set_vexpand(false);
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

        let wizard_title_entry = gtk::Entry::new();
        wizard_title_entry.set_hexpand(true);
        wizard_title_entry.set_editable(false);
        let title_sender = sender.clone();
        wizard_title_entry.connect_changed(move |entry| {
            title_sender.input(AppMsg::WizardTitleChanged(entry.text().to_string()));
        });
        let primary_sender = sender.clone();
        wizard_title_entry.connect_activate(move |_| {
            primary_sender.input(AppMsg::WizardPrimary);
        });

        let wizard_arch_dropdown = gtk::DropDown::from_strings(&[
            "64-bit WoW64 prefix (recommended)",
            "32-bit-only prefix (legacy Wine builds)",
        ]);
        let wizard_disc_dropdown = gtk::DropDown::from_strings(&[]);
        wizard_disc_dropdown.set_enable_search(true);
        let wizard_installer_dropdown = gtk::DropDown::from_strings(&[]);
        wizard_installer_dropdown.set_enable_search(true);
        let wizard_executable_dropdown = gtk::DropDown::from_strings(&[]);
        wizard_executable_dropdown.set_enable_search(true);

        let (application_data_root, library_store, games, library_error) = match data_root() {
            Ok(root) => {
                let store = LibraryStore::new(&root);
                match store.load() {
                    Ok(games) => (
                        Some(root),
                        Some(store),
                        games.into_iter().map(library_game_from_stored).collect(),
                        None,
                    ),
                    Err(error) => (Some(root), Some(store), Vec::new(), Some(error.to_string())),
                }
            }
            Err(error) => (None, None, Vec::new(), Some(error.to_string())),
        };

        let model = Self {
            page: Page::Library,
            games,
            library_error,
            library_edit_mode: false,
            query: String::new(),
            search: SearchState::Idle,
            vndb: VndbClient::new(),
            sender: sender.clone(),
            games_grid,
            query_entry,
            results_list,
            wizard: None,
            library_store,
            data_root: application_data_root,
            wizard_title_entry,
            wizard_arch_dropdown,
            wizard_disc_dropdown,
            wizard_installer_dropdown,
            wizard_executable_dropdown,
            search_target: None,
            install_target: None,
        };
        model.refresh_games_list();
        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, root: &Self::Root) {
        match message {
            AppMsg::ChooseSourceFile => {
                show_source_chooser(root, sender, gtk::FileChooserAction::Open);
            }
            AppMsg::ChooseGameFiles(id) => {
                self.install_target = Some(id);
                show_local_source_prompt(root, sender);
            }
            AppMsg::ChooseSourceFolder => {
                show_source_chooser(root, sender, gtk::FileChooserAction::SelectFolder);
            }
            AppMsg::SourceSelected(path) => {
                self.open_install_wizard(path);
            }
            AppMsg::WizardTitleChanged(title) => {
                if let Some(wizard) = &mut self.wizard {
                    wizard.title = title;
                    wizard.error = None;
                }
            }
            AppMsg::WizardPrimary => {
                self.advance_wizard(sender);
            }
            AppMsg::WizardBrowseExecutable => {
                self.show_executable_chooser(root, sender);
            }
            AppMsg::WizardExecutableSelected(path) => {
                if let Some(wizard) = &mut self.wizard {
                    wizard.selected_executable = Some(path);
                    wizard.error = None;
                }
            }
            AppMsg::SourcePrepared(result) => {
                self.source_prepared(result, sender);
            }
            AppMsg::SourceInspected { original, result } => {
                self.source_inspected(original, result, sender);
            }
            AppMsg::InstallationFinished { prepared, result } => {
                self.installation_finished(prepared, result);
            }
            AppMsg::ProfileSaved { outcome, result } => {
                self.profile_saved(outcome, result);
            }
            AppMsg::LaunchGame(id) => {
                self.start_game(id, sender);
            }
            AppMsg::GameFinished { title, result } => {
                if let Err(error) = result {
                    self.library_error = Some(format!("“{title}” stopped with an error: {error}"));
                }
            }
            AppMsg::ToggleLibraryEditMode => {
                self.library_edit_mode = !self.library_edit_mode;
                self.refresh_games_list();
            }
            AppMsg::RequestRemoveGame(id) => {
                if let Some(game) = self.games.iter().find(|game| game.id == id) {
                    show_remove_game_confirmation(
                        root,
                        sender,
                        id,
                        &game.metadata.title,
                        game.profile.is_some(),
                    );
                }
            }
            AppMsg::ConfirmRemoveGame(id) => {
                self.start_removing_game(id, false, sender);
            }
            AppMsg::ConfirmDeleteGameFiles(id) => {
                self.start_removing_game(id, true, sender);
            }
            AppMsg::GameRemovalFinished { id, result } => {
                self.game_removal_finished(id, result);
            }
            AppMsg::MoveGame { id, direction } => {
                self.move_game(id, direction);
            }
            AppMsg::AddVndbOnly(entry) => {
                self.add_vndb_game(entry, false, root, sender);
            }
            AppMsg::AddVndbAndFiles(entry) => {
                self.add_vndb_game(entry, true, root, sender);
            }
            AppMsg::EditMetadata(id) => {
                if let Some(game) = self.games.iter().find(|game| game.id == id) {
                    show_metadata_editor(root, sender, game);
                }
            }
            AppMsg::SearchMetadata(id) => {
                self.start_metadata_search(id);
            }
            AppMsg::SaveManualMetadata { id, metadata } => {
                self.save_manual_metadata(id, metadata);
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
                if let Some(target) = self.search_target {
                    self.replace_metadata_from_vndb(target, entry);
                } else {
                    show_vndb_add_choice(root, sender, entry);
                }
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

    fn search_heading(&self) -> &'static str {
        if self.search_target.is_some() {
            "Change metadata from VNDB"
        } else {
            "Add from VNDB"
        }
    }

    fn search_description(&self) -> &'static str {
        if self.search_target.is_some() {
            "Choose a result to replace this game’s title, release information, and VNDB ID."
        } else {
            "Choose a title, then add it to the library or select game files to install."
        }
    }

    fn search_text(&self) -> String {
        match &self.search {
            SearchState::Idle if self.search_target.is_some() => {
                "Search VNDB and choose the correct metadata.".to_owned()
            }
            SearchState::Idle => "Search VNDB to add a title.".to_owned(),
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
                let action = if self.search_target.is_some() {
                    "use"
                } else {
                    "add"
                };
                format!("{count} {noun} for “{query}”. Pick one to {action}.{suffix}")
            }
            SearchState::Failed { query, message } => {
                format!("Search for “{query}” failed.\n\n{message}")
            }
        }
    }

    fn back_tooltip(&self) -> &'static str {
        match self.page {
            Page::Install => "Cancel installation",
            _ => "Back to library",
        }
    }

    fn can_leave_page(&self) -> bool {
        self.page != Page::Install || !self.wizard_is_busy()
    }

    fn wizard_phase_is(&self, phase: WizardPhase) -> bool {
        self.wizard
            .as_ref()
            .is_some_and(|wizard| wizard.phase == phase)
    }

    fn wizard_is_busy(&self) -> bool {
        self.wizard.as_ref().is_some_and(|wizard| {
            matches!(
                wizard.phase,
                WizardPhase::Preparing
                    | WizardPhase::Inspecting
                    | WizardPhase::Installing
                    | WizardPhase::Saving
            )
        })
    }

    fn wizard_status_text(&self) -> String {
        let Some(wizard) = &self.wizard else {
            return String::new();
        };
        match wizard.phase {
            WizardPhase::Review => {
                "Review the source and prefix settings before files are changed.".to_owned()
            }
            WizardPhase::Preparing => {
                "Preparing the source. Archives and disc images can take several minutes.".to_owned()
            }
            WizardPhase::ChooseDisc => {
                "Multiple disc images were found. Choose the installer disc to continue.".to_owned()
            }
            WizardPhase::Inspecting => "Inspecting the selected installation media.".to_owned(),
            WizardPhase::ChooseInstaller => {
                "Multiple setup programs were found. Choose the one to run.".to_owned()
            }
            WizardPhase::Installing => {
                "The installer is running under Wine. Complete it in the installer window.".to_owned()
            }
            WizardPhase::ChooseExecutable if wizard.outcome.as_ref().is_some_and(|outcome| outcome.executables.is_empty()) => {
                "The installer finished, but no new game executable was detected. Choose it under drive_c.".to_owned()
            }
            WizardPhase::ChooseExecutable => {
                "Choose the installed game executable that the library should launch.".to_owned()
            }
            WizardPhase::Saving => "Saving the launch profile.".to_owned(),
        }
    }

    fn wizard_source_text(&self) -> String {
        self.wizard
            .as_ref()
            .map(|wizard| format!("{}\n{}", wizard.source.kind, wizard.source.path.display()))
            .unwrap_or_default()
    }

    fn wizard_primary_label(&self) -> &'static str {
        match self.wizard.as_ref().map(|wizard| wizard.phase) {
            Some(WizardPhase::Review) => "Prepare installation",
            Some(WizardPhase::ChooseDisc) => "Use selected disc",
            Some(WizardPhase::ChooseInstaller) => "Run selected installer",
            Some(WizardPhase::ChooseExecutable) => "Save launch profile",
            _ => "Working…",
        }
    }

    fn wizard_can_continue(&self) -> bool {
        let Some(wizard) = &self.wizard else {
            return false;
        };
        match wizard.phase {
            WizardPhase::Review => {
                !wizard.title.trim().is_empty()
                    && wizard.source.kind != SourceKind::OtherFile
                    && self.data_root.is_some()
                    && self.library_store.is_some()
            }
            WizardPhase::ChooseDisc => wizard
                .prepared_source
                .as_ref()
                .is_some_and(|prepared| !prepared.source.disc_images.is_empty()),
            WizardPhase::ChooseInstaller => wizard
                .prepared
                .as_ref()
                .is_some_and(|prepared| !prepared.installers.is_empty()),
            WizardPhase::ChooseExecutable => {
                wizard.selected_executable.is_some()
                    || wizard
                        .outcome
                        .as_ref()
                        .is_some_and(|outcome| !outcome.executables.is_empty())
            }
            _ => false,
        }
    }

    fn wizard_has_error(&self) -> bool {
        self.wizard
            .as_ref()
            .is_some_and(|wizard| wizard.error.is_some())
    }

    fn wizard_error_text(&self) -> String {
        self.wizard
            .as_ref()
            .and_then(|wizard| wizard.error.clone())
            .unwrap_or_default()
    }

    fn wizard_has_manual_executable(&self) -> bool {
        self.wizard
            .as_ref()
            .is_some_and(|wizard| wizard.selected_executable.is_some())
    }

    fn wizard_selected_executable_text(&self) -> String {
        self.wizard
            .as_ref()
            .and_then(|wizard| wizard.selected_executable.as_ref())
            .map(|path| format!("Selected: {}", path.display()))
            .unwrap_or_default()
    }

    fn open_install_wizard(&mut self, path: PathBuf) {
        let Some(title) = self.install_target.and_then(|id| {
            self.games
                .iter()
                .find(|game| game.id == id)
                .map(|game| game.metadata.title.clone())
        }) else {
            self.library_error = Some("Add the VNDB title before choosing game files.".to_owned());
            self.page = Page::Library;
            return;
        };
        match identify_source(path) {
            Ok(source) => {
                let title = title;
                let error = if source.kind == SourceKind::OtherFile {
                    Some("Choose a .7z, .rar, .zip, .iso, .mds, .exe, or a folder.".to_owned())
                } else if self.data_root.is_none() || self.library_store.is_none() {
                    Some(
                        "The application data directory is unavailable, so a profile cannot be saved."
                            .to_owned(),
                    )
                } else {
                    None
                };
                self.wizard = Some(InstallWizard {
                    source,
                    title: title.clone(),
                    phase: WizardPhase::Review,
                    prepared_source: None,
                    prepared: None,
                    outcome: None,
                    selected_executable: None,
                    error,
                });
                self.wizard_title_entry.set_text(&title);
                self.wizard_arch_dropdown.set_selected(0);
                set_path_choices(&self.wizard_disc_dropdown, &[]);
                set_path_choices(&self.wizard_installer_dropdown, &[]);
                set_path_choices(&self.wizard_executable_dropdown, &[]);
                self.library_error = None;
                self.page = Page::Install;
            }
            Err(error) => {
                self.library_error = Some(error.to_string());
                self.page = Page::Library;
            }
        }
    }

    fn advance_wizard(&mut self, sender: ComponentSender<App>) {
        let Some(phase) = self.wizard.as_ref().map(|wizard| wizard.phase) else {
            return;
        };
        match phase {
            WizardPhase::Review => self.start_preparing_source(sender),
            WizardPhase::ChooseDisc => {
                let Some(prepared) = self
                    .wizard
                    .as_mut()
                    .and_then(|wizard| wizard.prepared_source.take())
                else {
                    return;
                };
                let selected =
                    selected_path(&prepared.source.disc_images, &self.wizard_disc_dropdown);
                self.start_inspecting_source(prepared, selected, sender);
            }
            WizardPhase::ChooseInstaller => {
                let Some(prepared) = self
                    .wizard
                    .as_mut()
                    .and_then(|wizard| wizard.prepared.take())
                else {
                    return;
                };
                let Some(installer) =
                    selected_path(&prepared.installers, &self.wizard_installer_dropdown)
                else {
                    if let Some(wizard) = &mut self.wizard {
                        wizard.prepared = Some(prepared);
                        wizard.error = Some("Choose an installer to continue.".to_owned());
                    }
                    return;
                };
                self.start_installer(prepared, installer, sender);
            }
            WizardPhase::ChooseExecutable => self.start_saving_profile(sender),
            _ => {}
        }
    }

    fn start_preparing_source(&mut self, sender: ComponentSender<App>) {
        let Some(wizard) = &mut self.wizard else {
            return;
        };
        let Some(data_root) = self.data_root.clone() else {
            wizard.error = Some("The application data directory is unavailable.".to_owned());
            return;
        };
        if wizard.title.trim().is_empty() {
            wizard.error = Some("Enter a game title.".to_owned());
            return;
        }
        if let Some(prepared) = wizard.prepared_source.take() {
            prepared.discard();
        }
        if let Some(prepared) = wizard.prepared.take() {
            prepared.discard();
        }
        let request = InstallRequest {
            title: wizard.title.trim().to_owned(),
            source: wizard.source.clone(),
            arch: if self.wizard_arch_dropdown.selected() == 1 {
                PrefixArch::Win32
            } else {
                PrefixArch::Win64
            },
            locale: "ja_JP.UTF-8".to_owned(),
            data_root,
            commands: RuntimeCommands::default(),
        };
        wizard.phase = WizardPhase::Preparing;
        wizard.error = None;

        thread::spawn(move || {
            let result = prepare_install_source(request);
            sender.input(AppMsg::SourcePrepared(result));
        });
    }

    fn source_prepared(
        &mut self,
        result: Result<PreparedInstallSource, InstallError>,
        sender: ComponentSender<App>,
    ) {
        match result {
            Ok(prepared) if prepared.source.disc_images.len() > 1 => {
                set_path_choices(&self.wizard_disc_dropdown, &prepared.source.disc_images);
                if let Some(wizard) = &mut self.wizard {
                    wizard.phase = WizardPhase::ChooseDisc;
                    wizard.prepared_source = Some(prepared);
                }
            }
            Ok(prepared) => {
                let selected = prepared.source.disc_images.first().cloned();
                self.start_inspecting_source(prepared, selected, sender);
            }
            Err(error) => {
                if let Some(wizard) = &mut self.wizard {
                    wizard.phase = WizardPhase::Review;
                    wizard.error = Some(error.to_string());
                }
            }
        }
    }

    fn start_inspecting_source(
        &mut self,
        prepared: PreparedInstallSource,
        selected_disc: Option<PathBuf>,
        sender: ComponentSender<App>,
    ) {
        if let Some(wizard) = &mut self.wizard {
            wizard.phase = WizardPhase::Inspecting;
            wizard.prepared_source = None;
            wizard.error = None;
        }
        thread::spawn(move || {
            let original = prepared.clone();
            let result = inspect_install_source(prepared, selected_disc);
            sender.input(AppMsg::SourceInspected { original, result });
        });
    }

    fn source_inspected(
        &mut self,
        original: PreparedInstallSource,
        result: Result<PreparedInstall, InstallError>,
        sender: ComponentSender<App>,
    ) {
        match result {
            Ok(prepared) if prepared.installers.len() == 1 => {
                let installer = prepared.installers[0].clone();
                self.start_installer(prepared, installer, sender);
            }
            Ok(prepared) => {
                set_path_choices(&self.wizard_installer_dropdown, &prepared.installers);
                if let Some(wizard) = &mut self.wizard {
                    wizard.phase = WizardPhase::ChooseInstaller;
                    wizard.prepared = Some(prepared);
                    wizard.error = None;
                }
            }
            Err(error) => {
                let can_choose_another_disc = original.source.disc_images.len() > 1;
                if let Some(wizard) = &mut self.wizard {
                    wizard.phase = if can_choose_another_disc {
                        WizardPhase::ChooseDisc
                    } else {
                        WizardPhase::Review
                    };
                    wizard.prepared_source = Some(original);
                    wizard.error = Some(error.to_string());
                }
            }
        }
    }

    fn start_installer(
        &mut self,
        prepared: PreparedInstall,
        installer: PathBuf,
        sender: ComponentSender<App>,
    ) {
        if let Some(wizard) = &mut self.wizard {
            wizard.phase = WizardPhase::Installing;
            wizard.prepared = None;
            wizard.error = None;
        }
        thread::spawn(move || {
            let original = prepared.clone();
            let result = execute_install(prepared, installer);
            sender.input(AppMsg::InstallationFinished {
                prepared: original,
                result,
            });
        });
    }

    fn installation_finished(
        &mut self,
        prepared: PreparedInstall,
        result: Result<InstallOutcome, InstallError>,
    ) {
        match result {
            Ok(outcome) => {
                set_path_choices(&self.wizard_executable_dropdown, &outcome.executables);
                if let Some(wizard) = &mut self.wizard {
                    wizard.phase = WizardPhase::ChooseExecutable;
                    wizard.prepared = None;
                    wizard.selected_executable = None;
                    wizard.error = if outcome.executables.is_empty() {
                        Some(
                            "No new .exe was found outside drive_c/windows. Choose the installed game executable manually."
                                .to_owned(),
                        )
                    } else {
                        None
                    };
                    wizard.outcome = Some(outcome);
                }
            }
            Err(error) => {
                set_path_choices(&self.wizard_installer_dropdown, &prepared.installers);
                if let Some(wizard) = &mut self.wizard {
                    wizard.phase = WizardPhase::ChooseInstaller;
                    wizard.prepared = Some(prepared);
                    wizard.error = Some(error.to_string());
                }
            }
        }
    }

    fn start_saving_profile(&mut self, sender: ComponentSender<App>) {
        let Some(wizard) = &mut self.wizard else {
            return;
        };
        let Some(outcome) = wizard.outcome.take() else {
            return;
        };
        let executable = wizard
            .selected_executable
            .clone()
            .or_else(|| selected_path(&outcome.executables, &self.wizard_executable_dropdown));
        let Some(executable) = executable else {
            wizard.outcome = Some(outcome);
            wizard.error = Some("Choose the installed game executable.".to_owned());
            return;
        };
        let profile = match outcome.launch_profile(executable) {
            Ok(profile) => profile,
            Err(error) => {
                wizard.outcome = Some(outcome);
                wizard.error = Some(error.to_string());
                return;
            }
        };
        let Some(store) = self.library_store.clone() else {
            wizard.outcome = Some(outcome);
            wizard.error = Some("The library database is unavailable.".to_owned());
            return;
        };
        let Some(target) = self.install_target else {
            wizard.outcome = Some(outcome);
            wizard.error = Some("The target library game is unavailable.".to_owned());
            return;
        };
        wizard.phase = WizardPhase::Saving;
        wizard.error = None;

        thread::spawn(move || {
            let result = store.attach_profile(target.0, &profile);
            sender.input(AppMsg::ProfileSaved { outcome, result });
        });
    }

    fn profile_saved(
        &mut self,
        outcome: InstallOutcome,
        result: Result<StoredProfile, LibraryError>,
    ) {
        match result {
            Ok(stored) => {
                outcome.cleanup_after_save();
                if let Some(target) = self.install_target
                    && let Some(game) = self.games.iter_mut().find(|game| game.id == target)
                {
                    game.profile = Some(stored);
                }
                self.install_target = None;
                self.wizard = None;
                self.library_error = None;
                self.refresh_games_list();
                self.page = Page::Library;
            }
            Err(error) => {
                if let Some(wizard) = &mut self.wizard {
                    wizard.phase = WizardPhase::ChooseExecutable;
                    wizard.outcome = Some(outcome);
                    wizard.error = Some(error.to_string());
                }
            }
        }
    }

    fn show_executable_chooser(&self, root: &adw::ApplicationWindow, sender: ComponentSender<App>) {
        let Some(outcome) = self
            .wizard
            .as_ref()
            .filter(|wizard| wizard.phase == WizardPhase::ChooseExecutable)
            .and_then(|wizard| wizard.outcome.as_ref())
        else {
            return;
        };
        show_executable_chooser(root, sender, &outcome.prefix.join("drive_c"));
    }

    fn add_vndb_game(
        &mut self,
        entry: SearchResultWithThumbnail,
        choose_files: bool,
        root: &adw::ApplicationWindow,
        sender: ComponentSender<App>,
    ) {
        let Some(store) = &self.library_store else {
            self.library_error = Some("The library database is unavailable.".to_owned());
            self.show_library();
            return;
        };
        let metadata = metadata_from_vndb(&entry.summary);
        match store.add_game(&metadata, entry.thumbnail.as_deref()) {
            Ok(stored) => {
                let id = LibraryGameId(stored.id);
                self.games.push(library_game_from_stored(stored));
                self.library_error = None;
                self.refresh_games_list();
                if choose_files {
                    self.install_target = Some(id);
                    self.show_library();
                    self.install_target = Some(id);
                    show_local_source_prompt(root, sender);
                } else {
                    self.show_library();
                }
            }
            Err(error) => {
                self.library_error = Some(error.to_string());
                self.show_library();
            }
        }
    }

    fn start_metadata_search(&mut self, id: LibraryGameId) {
        let Some(game) = self.games.iter().find(|game| game.id == id) else {
            return;
        };
        self.search_target = Some(id);
        self.query_entry.set_text(&game.metadata.title);
        self.query = game.metadata.title.clone();
        self.search = SearchState::Idle;
        self.replace_search_results(Vec::new());
        self.library_edit_mode = false;
        self.refresh_games_list();
        self.page = Page::Search;
        self.library_error = None;
    }

    fn replace_metadata_from_vndb(&mut self, id: LibraryGameId, entry: SearchResultWithThumbnail) {
        let metadata = metadata_from_vndb(&entry.summary);
        self.update_game_metadata(id, metadata, entry.thumbnail);
        self.search_target = None;
        self.show_library();
    }

    fn save_manual_metadata(&mut self, id: LibraryGameId, metadata: GameMetadata) {
        let thumbnail = self
            .games
            .iter()
            .find(|game| game.id == id)
            .and_then(|game| game.thumbnail.clone());
        self.update_game_metadata(id, metadata, thumbnail);
    }

    fn update_game_metadata(
        &mut self,
        id: LibraryGameId,
        metadata: GameMetadata,
        thumbnail: Option<Vec<u8>>,
    ) {
        let Some(store) = &self.library_store else {
            self.library_error = Some("The library database is unavailable.".to_owned());
            return;
        };
        match store.update_metadata(id.0, &metadata, thumbnail.as_deref()) {
            Ok(()) => {
                if let Some(game) = self.games.iter_mut().find(|game| game.id == id) {
                    game.metadata = metadata;
                    game.thumbnail = thumbnail;
                    if let Some(profile) = &mut game.profile {
                        profile.title = game.metadata.title.clone();
                        profile.profile.vndb_id = game.metadata.vndb_id.clone();
                    }
                }
                self.library_error = None;
                self.refresh_games_list();
            }
            Err(error) => {
                self.library_error = Some(error.to_string());
            }
        }
    }

    fn start_game(&mut self, id: LibraryGameId, sender: ComponentSender<App>) {
        let Some(game) = self.games.iter().find(|game| game.id == id) else {
            return;
        };
        let Some(stored) = &game.profile else {
            self.library_error =
                Some("Choose game files before trying to play this title.".to_owned());
            return;
        };
        let Some(data_root) = self.data_root.clone() else {
            self.library_error = Some("The application data directory is unavailable.".to_owned());
            return;
        };
        let profile = stored.profile.clone();
        let title = game.metadata.title.clone();
        self.library_error = None;
        thread::spawn(move || {
            let result = run_game(&profile, &data_root, &RuntimeCommands::default());
            sender.input(AppMsg::GameFinished { title, result });
        });
    }

    fn start_removing_game(
        &mut self,
        id: LibraryGameId,
        delete_files: bool,
        sender: ComponentSender<App>,
    ) {
        let Some(game) = self.games.iter().find(|game| game.id == id) else {
            return;
        };
        let Some(store) = self.library_store.clone() else {
            self.library_error = Some("The library database is unavailable.".to_owned());
            return;
        };
        let prefix = game
            .profile
            .as_ref()
            .map(|stored| stored.profile.prefix.clone());
        let data_root = self.data_root.clone();
        self.library_error = None;
        thread::spawn(move || {
            let result = remove_stored_game(
                &store,
                id,
                prefix.as_deref(),
                data_root.as_deref(),
                delete_files,
            );
            sender.input(AppMsg::GameRemovalFinished { id, result });
        });
    }

    fn game_removal_finished(&mut self, id: LibraryGameId, result: Result<Option<String>, String>) {
        match result {
            Ok(warning) => {
                self.games.retain(|game| game.id != id);
                if self.games.is_empty() {
                    self.library_edit_mode = false;
                }
                self.library_error = warning;
                self.refresh_games_list();
            }
            Err(error) => {
                self.library_error = Some(error);
            }
        }
    }

    fn move_game(&mut self, id: LibraryGameId, direction: MoveDirection) {
        let Some(index) = self.games.iter().position(|game| game.id == id) else {
            return;
        };
        let Some(target) = moved_index(self.games.len(), index, direction) else {
            return;
        };
        self.games.swap(index, target);

        if let Some(store) = &self.library_store {
            let ids: Vec<i64> = self.games.iter().map(|game| game.id.0).collect();
            if let Err(error) = store.reorder(&ids) {
                self.games.swap(index, target);
                self.library_error = Some(error.to_string());
                self.refresh_games_list();
                return;
            }
        }

        self.library_error = None;
        self.refresh_games_list();
    }

    fn show_search(&mut self) {
        self.search_target = None;
        self.library_edit_mode = false;
        self.refresh_games_list();
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
        if self.wizard_is_busy() {
            return;
        }
        if let Some(wizard) = self.wizard.take() {
            wizard.discard();
            self.install_target = None;
        }
        self.search_target = None;
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

        for (index, game) in self.games.iter().enumerate() {
            self.games_grid.append(&build_game_card(
                game,
                self.sender.clone(),
                self.library_edit_mode,
                index > 0,
                index + 1 < self.games.len(),
            ));
            if let Some(child) = self.games_grid.last_child() {
                child.set_vexpand(false);
                child.set_valign(gtk::Align::Start);
            }
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

impl InstallWizard {
    fn discard(self) {
        if let Some(outcome) = self.outcome {
            outcome.discard();
        } else if let Some(prepared) = self.prepared {
            prepared.discard();
        } else if let Some(prepared) = self.prepared_source {
            prepared.discard();
        }
    }
}

fn set_path_choices(dropdown: &gtk::DropDown, paths: &[PathBuf]) {
    let labels: Vec<String> = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    let references: Vec<&str> = labels.iter().map(String::as_str).collect();
    let model = gtk::StringList::new(&references);
    dropdown.set_model(Some(&model));
    dropdown.set_selected(if paths.is_empty() {
        gtk::INVALID_LIST_POSITION
    } else {
        0
    });
    dropdown.set_sensitive(!paths.is_empty());
}

fn selected_path(paths: &[PathBuf], dropdown: &gtk::DropDown) -> Option<PathBuf> {
    paths.get(dropdown.selected() as usize).cloned()
}
fn remove_stored_game(
    store: &LibraryStore,
    id: LibraryGameId,
    prefix: Option<&Path>,
    data_root: Option<&Path>,
    delete_files: bool,
) -> Result<Option<String>, String> {
    if !delete_files || prefix.is_none() {
        return store
            .delete(id.0)
            .map(|()| None)
            .map_err(|error| error.to_string());
    }

    let prefix = prefix.expect("checked above");
    if !prefix.exists() {
        return store
            .delete(id.0)
            .map(|()| None)
            .map_err(|error| error.to_string());
    }
    let data_root = data_root.ok_or_else(|| {
        "The application data directory is unavailable; no game files were deleted.".to_owned()
    })?;
    let managed_root = data_root
        .join("prefixes")
        .canonicalize()
        .map_err(|error| format!("Cannot validate the managed prefix directory: {error}"))?;
    let canonical_prefix = prefix.canonicalize().map_err(|error| {
        format!(
            "Cannot validate the game prefix {}: {error}",
            prefix.display()
        )
    })?;
    if canonical_prefix.parent() != Some(managed_root.as_path()) {
        return Err(format!(
            "Refusing to delete files outside ryoiki’s managed prefix directory: {}",
            canonical_prefix.display()
        ));
    }

    let staged = managed_root.join(format!(".deleting-{}", id.0));
    if staged.exists() {
        return Err(format!(
            "Cannot stage prefix deletion because {} already exists.",
            staged.display()
        ));
    }
    fs::rename(&canonical_prefix, &staged).map_err(|error| {
        format!(
            "Cannot stage game files at {} for deletion: {error}",
            canonical_prefix.display()
        )
    })?;

    if let Err(error) = store.delete(id.0) {
        let rollback = fs::rename(&staged, &canonical_prefix);
        return Err(match rollback {
            Ok(()) => error.to_string(),
            Err(rollback_error) => format!(
                "{error}. Restoring the prefix also failed: {rollback_error}. Files remain at {}.",
                staged.display()
            ),
        });
    }

    Ok(fs::remove_dir_all(&staged).err().map(|error| {
        format!(
            "Removed the library entry, but could not finish deleting {}: {error}",
            staged.display()
        )
    }))
}

fn moved_index(length: usize, index: usize, direction: MoveDirection) -> Option<usize> {
    match direction {
        MoveDirection::Previous => index.checked_sub(1),
        MoveDirection::Next if index + 1 < length => Some(index + 1),
        MoveDirection::Next => None,
    }
}

fn library_game_from_stored(stored: StoredGame) -> LibraryGame {
    LibraryGame {
        id: LibraryGameId(stored.id),
        metadata: stored.metadata,
        profile: stored.profile,
        thumbnail: stored.thumbnail,
    }
}

fn metadata_from_vndb(summary: &VnSummary) -> GameMetadata {
    GameMetadata {
        title: summary.title.clone(),
        alttitle: summary.alttitle.clone(),
        released: summary.released.clone(),
        vndb_id: Some(summary.id.clone()),
    }
}

fn show_vndb_add_choice(
    root: &adw::ApplicationWindow,
    sender: ComponentSender<App>,
    entry: SearchResultWithThumbnail,
) {
    let window = gtk::Window::builder()
        .transient_for(root)
        .modal(true)
        .title("Add game")
        .resizable(false)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_all(18);
    let heading = gtk::Label::new(Some(&format!("Add “{}”?", entry.summary.title)));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    content.append(&heading);
    let description = gtk::Label::new(Some(
        "Add the VNDB metadata now. Game files can be selected immediately or from the library later.",
    ));
    description.set_xalign(0.0);
    description.set_wrap(true);
    content.append(&description);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let add_only = gtk::Button::with_label("Add to library");
    let add_files = gtk::Button::with_label("Choose game files");
    add_files.add_css_class("suggested-action");
    let cancel_window = window.clone();
    cancel.connect_clicked(move |_| cancel_window.close());
    let only_window = window.clone();
    let only_sender = sender.clone();
    let only_entry = entry.clone();
    add_only.connect_clicked(move |_| {
        only_window.close();
        only_sender.input(AppMsg::AddVndbOnly(only_entry.clone()));
    });
    let files_window = window.clone();
    add_files.connect_clicked(move |_| {
        files_window.close();
        sender.input(AppMsg::AddVndbAndFiles(entry.clone()));
    });
    buttons.append(&cancel);
    buttons.append(&add_only);
    buttons.append(&add_files);
    content.append(&buttons);
    window.set_child(Some(&content));
    window.present();
}

fn show_metadata_editor(
    root: &adw::ApplicationWindow,
    sender: ComponentSender<App>,
    game: &LibraryGame,
) {
    let window = gtk::Window::builder()
        .transient_for(root)
        .modal(true)
        .title("Edit game metadata")
        .resizable(false)
        .default_width(460)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_all(18);

    let title = gtk::Entry::new();
    title.set_text(&game.metadata.title);
    title.set_placeholder_text(Some("Display title"));
    content.append(&gtk::Label::builder().label("Title").xalign(0.0).build());
    content.append(&title);

    let original_title = gtk::Entry::new();
    original_title.set_text(game.metadata.alttitle.as_deref().unwrap_or_default());
    original_title.set_placeholder_text(Some("Optional original title"));
    content.append(
        &gtk::Label::builder()
            .label("Original title")
            .xalign(0.0)
            .build(),
    );
    content.append(&original_title);

    let released = gtk::Entry::new();
    released.set_text(game.metadata.released.as_deref().unwrap_or_default());
    released.set_placeholder_text(Some("Optional release date"));
    content.append(
        &gtk::Label::builder()
            .label("Release date")
            .xalign(0.0)
            .build(),
    );
    content.append(&released);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let search = gtk::Button::with_label("Search VNDB");
    let save = gtk::Button::with_label("Save");
    save.add_css_class("suggested-action");
    save.set_sensitive(!title.text().trim().is_empty());
    let save_for_title = save.clone();
    title.connect_changed(move |entry| {
        save_for_title.set_sensitive(!entry.text().trim().is_empty())
    });

    let cancel_window = window.clone();
    cancel.connect_clicked(move |_| cancel_window.close());
    let search_window = window.clone();
    let search_sender = sender.clone();
    let id = game.id;
    search.connect_clicked(move |_| {
        search_window.close();
        search_sender.input(AppMsg::SearchMetadata(id));
    });
    let save_window = window.clone();
    let vndb_id = game.metadata.vndb_id.clone();
    save.connect_clicked(move |_| {
        save_window.close();
        sender.input(AppMsg::SaveManualMetadata {
            id,
            metadata: GameMetadata {
                title: title.text().trim().to_owned(),
                alttitle: nonempty_entry(&original_title),
                released: nonempty_entry(&released),
                vndb_id: vndb_id.clone(),
            },
        });
    });

    buttons.append(&cancel);
    buttons.append(&search);
    buttons.append(&save);
    content.append(&buttons);
    window.set_child(Some(&content));
    window.present();
}

fn nonempty_entry(entry: &gtk::Entry) -> Option<String> {
    let value = entry.text().trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn show_remove_game_confirmation(
    root: &adw::ApplicationWindow,
    sender: ComponentSender<App>,
    id: LibraryGameId,
    title: &str,
    has_game_files: bool,
) {
    let window = gtk::Window::builder()
        .transient_for(root)
        .modal(true)
        .title("Remove game")
        .resizable(false)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_all(18);
    let heading = gtk::Label::new(Some(&format!("Remove “{title}”?")));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    content.append(&heading);
    let description = gtk::Label::new(Some(if has_game_files {
        "Remove only the library entry, or also permanently delete its ryoiki-managed Wine prefix. Original archives, disc images, and external files are never deleted."
    } else {
        "This removes the metadata-only library entry."
    }));
    description.set_xalign(0.0);
    description.set_wrap(true);
    content.append(&description);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let remove = gtk::Button::with_label("Remove from library");
    let cancel_window = window.clone();
    cancel.connect_clicked(move |_| cancel_window.close());
    let remove_window = window.clone();
    let remove_sender = sender.clone();
    remove.connect_clicked(move |_| {
        remove_window.close();
        remove_sender.input(AppMsg::ConfirmRemoveGame(id));
    });
    buttons.append(&cancel);
    buttons.append(&remove);
    if has_game_files {
        let delete = gtk::Button::with_label("Delete files and remove");
        delete.add_css_class("destructive-action");
        let delete_window = window.clone();
        delete.connect_clicked(move |_| {
            delete_window.close();
            sender.input(AppMsg::ConfirmDeleteGameFiles(id));
        });
        buttons.append(&delete);
    }
    content.append(&buttons);
    window.set_child(Some(&content));
    window.present();
}

fn show_executable_chooser(
    root: &adw::ApplicationWindow,
    sender: ComponentSender<App>,
    drive_c: &Path,
) {
    let chooser = gtk::FileChooserNative::new(
        Some("Choose installed game executable"),
        Some(root),
        gtk::FileChooserAction::Open,
        Some("Choose"),
        Some("Cancel"),
    );
    let executables = gtk::FileFilter::new();
    executables.set_name(Some("Windows executables"));
    executables.add_pattern("*.exe");
    executables.add_pattern("*.EXE");
    chooser.add_filter(&executables);
    let folder = gtk::gio::File::for_path(drive_c);
    let _ = chooser.set_current_folder(Some(&folder));
    chooser.connect_response(move |chooser, response| {
        if response == gtk::ResponseType::Accept
            && let Some(path) = chooser.file().and_then(|file| file.path())
        {
            sender.input(AppMsg::WizardExecutableSelected(path));
        }
        chooser.destroy();
    });
    chooser.show();
}

fn show_local_source_prompt(root: &adw::ApplicationWindow, sender: ComponentSender<App>) {
    let window = gtk::Window::builder()
        .transient_for(root)
        .modal(true)
        .title("Install from local files")
        .resizable(false)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_all(18);

    let description = gtk::Label::new(Some(
        "Choose an archive, disc image, installer, or folder. The wizard will prepare a separate Wine prefix and run the installer.",
    ));
    description.set_wrap(true);
    description.set_xalign(0.0);
    description.add_css_class("dim-label");
    content.append(&description);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);

    let cancel = gtk::Button::with_label("Cancel");
    let archive = gtk::Button::with_label("File");
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
        _ => "Choose install files",
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
        supported.set_name(Some("Install sources"));
        for pattern in [
            "*.7z", "*.rar", "*.zip", "*.iso", "*.mds", "*.exe", "*.7Z", "*.RAR", "*.ZIP", "*.ISO",
            "*.MDS", "*.EXE",
        ] {
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

fn build_game_card(
    game: &LibraryGame,
    sender: ComponentSender<App>,
    edit_mode: bool,
    can_move_previous: bool,
    can_move_next: bool,
) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class("card");
    card.add_css_class("library-card");
    card.set_hexpand(false);
    card.set_vexpand(false);
    card.set_halign(gtk::Align::Fill);
    card.set_valign(gtk::Align::Start);
    card.set_size_request(196, -1);

    card.append(&cover_widget(
        game.thumbnail.clone(),
        &format!("Cover for {}", game.metadata.title),
        CARD_COVER_WIDTH,
        CARD_COVER_HEIGHT,
        "library-cover",
    ));

    let title = gtk::Label::new(Some(&game.metadata.title));
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_justify(gtk::Justification::Center);
    title.set_xalign(0.5);
    title.set_max_width_chars(18);
    title.set_lines(2);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.add_css_class("heading");
    title.add_css_class("library-title");
    title.set_hexpand(true);
    title.set_vexpand(false);
    title.set_valign(gtk::Align::Start);
    title.set_yalign(0.5);
    card.append(&title);

    if !edit_mode {
        let action = if game.profile.is_some() {
            let play = gtk::Button::with_label("Play");
            play.add_css_class("suggested-action");
            let id = game.id;
            play.connect_clicked(move |_| sender.input(AppMsg::LaunchGame(id)));
            play
        } else {
            let files = gtk::Button::with_label("Choose game files");
            let id = game.id;
            files.connect_clicked(move |_| sender.input(AppMsg::ChooseGameFiles(id)));
            files
        };
        action.add_css_class("library-action");
        action.set_hexpand(true);
        action.set_vexpand(false);
        action.set_valign(gtk::Align::Start);
        if let Some(label) = action.child().and_downcast::<gtk::Label>() {
            label.set_wrap(false);
            label.set_single_line_mode(true);
        }
        card.append(&action);
        card.append(&library_card_spacer());
        return wrap_library_card(card);
    }
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    controls.set_halign(gtk::Align::Center);
    controls.set_hexpand(true);
    controls.set_vexpand(false);
    controls.set_valign(gtk::Align::End);

    let previous = gtk::Button::from_icon_name("go-previous-symbolic");
    previous.set_tooltip_text(Some("Move earlier"));
    previous.set_sensitive(can_move_previous);
    let previous_sender = sender.clone();
    let id = game.id;
    previous.connect_clicked(move |_| {
        previous_sender.input(AppMsg::MoveGame {
            id,
            direction: MoveDirection::Previous,
        });
    });
    controls.append(&previous);

    let next = gtk::Button::from_icon_name("go-next-symbolic");
    next.set_tooltip_text(Some("Move later"));
    next.set_sensitive(can_move_next);
    let next_sender = sender.clone();
    let id = game.id;
    next.connect_clicked(move |_| {
        next_sender.input(AppMsg::MoveGame {
            id,
            direction: MoveDirection::Next,
        });
    });
    controls.append(&next);

    let edit = gtk::Button::from_icon_name("document-edit-symbolic");
    edit.set_tooltip_text(Some("Edit metadata"));
    let edit_sender = sender.clone();
    let id = game.id;
    edit.connect_clicked(move |_| edit_sender.input(AppMsg::EditMetadata(id)));
    controls.append(&edit);

    let remove = gtk::Button::from_icon_name("user-trash-symbolic");
    remove.set_tooltip_text(Some("Remove or delete game files"));
    remove.add_css_class("destructive-action");
    let id = game.id;
    remove.connect_clicked(move |_| sender.input(AppMsg::RequestRemoveGame(id)));
    controls.append(&remove);
    card.append(&controls);
    card.append(&library_card_spacer());

    wrap_library_card(card)
}

fn wrap_library_card(card: gtk::Box) -> gtk::Box {
    let wrapper = gtk::Box::new(gtk::Orientation::Vertical, 0);
    wrapper.set_hexpand(true);
    wrapper.set_vexpand(false);
    wrapper.set_halign(gtk::Align::Fill);
    wrapper.set_valign(gtk::Align::Start);
    wrapper.append(&card);
    wrapper
}

fn library_card_spacer() -> gtk::Box {
    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    spacer.set_hexpand(false);
    spacer
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
    let slot = gtk::Fixed::new();
    apply_cover_slot(&slot, width, height, css_class);
    slot.set_overflow(gtk::Overflow::Hidden);

    if let Some(texture) = thumbnail.and_then(|bytes| thumbnail_texture(bytes, width, height)) {
        let picture = gtk::Picture::for_paintable(&texture);
        picture.set_alternative_text(Some(alternative_text));
        picture.set_can_shrink(true);
        picture.set_keep_aspect_ratio(false);
        picture.set_size_request(width, height);
        slot.put(&picture, 0.0, 0.0);
    } else {
        let image = gtk::Image::from_icon_name("image-missing-symbolic");
        image.set_pixel_size(32);
        image.add_css_class("dim-label");
        image.set_size_request(width, height);
        slot.put(&image, 0.0, 0.0);
    }

    slot.upcast()
}

fn thumbnail_texture(thumbnail: Vec<u8>, width: i32, height: i32) -> Option<gtk::gdk::Texture> {
    let stream = gtk::gio::MemoryInputStream::from_bytes(&gtk::glib::Bytes::from_owned(thumbnail));
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_stream(&stream, gtk::gio::Cancellable::NONE).ok()?;
    let (x, y, crop_width, crop_height) = cover_crop_region(pixbuf.width(), pixbuf.height(), 2, 3);
    let cropped = pixbuf.new_subpixbuf(x, y, crop_width, crop_height);
    let scaled = cropped.scale_simple(width, height, gtk::gdk_pixbuf::InterpType::Bilinear)?;
    Some(gtk::gdk::Texture::for_pixbuf(&scaled))
}

fn apply_cover_slot(
    widget: &impl gtk::prelude::WidgetExt,
    width: i32,
    height: i32,
    css_class: &str,
) {
    widget.set_size_request(width, height);
    widget.set_hexpand(false);
    widget.set_vexpand(false);
    widget.set_halign(gtk::Align::Center);
    widget.set_valign(gtk::Align::Start);
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        profile::{GameMetadata, LaunchProfile, PrefixArch, Runner, StoredProfile},
        vndb::VnSummary,
    };
    use relm4::gtk::{self, prelude::*};
    use tempfile::tempdir;

    use super::{LibraryGameId, MoveDirection, moved_index, remove_stored_game, result_subtitle};

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
        super::apply_cover_slot(
            &image,
            super::COVER_WIDTH,
            super::COVER_HEIGHT,
            "vndb-cover",
        );
        assert_eq!(image.width_request(), super::COVER_WIDTH);
        assert_eq!(image.height_request(), super::COVER_HEIGHT);
        assert!(image.has_css_class("vndb-cover"));

        let pixbuf =
            gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 400, 200)
                .expect("pixbuf");
        pixbuf.fill(0x3366_99ff);
        let png = pixbuf.save_to_bufferv("png", &[]).expect("encode png");
        let texture = super::thumbnail_texture(png.clone(), super::COVER_WIDTH, super::COVER_HEIGHT)
            .expect("decode cover");
        assert_eq!(texture.width(), super::COVER_WIDTH);
        assert_eq!(texture.height(), super::COVER_HEIGHT);

        let missing = super::cover_widget(
            None,
            "Cover for Missing",
            super::CARD_COVER_WIDTH,
            super::CARD_COVER_HEIGHT,
            "library-cover",
        );
        assert_eq!(missing.width_request(), super::CARD_COVER_WIDTH);
        assert_eq!(missing.height_request(), super::CARD_COVER_HEIGHT);
        assert!(missing.has_css_class("library-cover"));

        let present = super::cover_widget(
            Some(png),
            "Cover for Present",
            super::CARD_COVER_WIDTH,
            super::CARD_COVER_HEIGHT,
            "library-cover",
        );
        assert_eq!(present.width_request(), super::CARD_COVER_WIDTH);
        assert_eq!(present.height_request(), super::CARD_COVER_HEIGHT);
        assert!(present.has_css_class("library-cover"));
    }

    #[test]
    fn removal_can_preserve_or_delete_only_managed_prefixes() {
        let root = tempdir().expect("temporary directory");
        let store = crate::profile::LibraryStore::new(root.path());
        let prefixes = root.path().join("prefixes");
        let prefix = prefixes.join("game");
        std::fs::create_dir_all(prefix.join("drive_c")).expect("managed prefix");
        let metadata = GameMetadata {
            title: "Game".to_owned(),
            alttitle: None,
            released: None,
            vndb_id: Some("v1".to_owned()),
        };
        let profile = LaunchProfile {
            exe: prefix.join("drive_c/game.exe"),
            prefix: prefix.clone(),
            arch: PrefixArch::Win64,
            runner: Runner::Wine,
            locale: "ja_JP.UTF-8".to_owned(),
            disc: None,
            winetricks: Vec::new(),
            vndb_id: Some("v1".to_owned()),
            notes: String::new(),
        };

        let first = store.add_game(&metadata, None).expect("add first game");
        store
            .attach_profile(first.id, &profile)
            .expect("attach first profile");
        remove_stored_game(
            &store,
            LibraryGameId(first.id),
            Some(&prefix),
            Some(root.path()),
            false,
        )
        .expect("remove library entry only");
        assert!(prefix.is_dir());
        assert!(store.load().expect("load empty library").is_empty());

        let second = store.add_game(&metadata, None).expect("add second game");
        store
            .attach_profile(second.id, &profile)
            .expect("attach second profile");
        remove_stored_game(
            &store,
            LibraryGameId(second.id),
            Some(&prefix),
            Some(root.path()),
            true,
        )
        .expect("delete managed prefix");
        assert!(!prefix.exists());
        assert!(store.load().expect("load empty library").is_empty());
    }

    #[test]
    fn destructive_removal_rejects_external_prefixes() {
        let root = tempdir().expect("temporary directory");
        let external = tempdir().expect("external prefix");
        std::fs::create_dir(root.path().join("prefixes")).expect("managed prefix root");
        let store = crate::profile::LibraryStore::new(root.path());
        let game = store
            .add_game(
                &GameMetadata {
                    title: "External".to_owned(),
                    alttitle: None,
                    released: None,
                    vndb_id: None,
                },
                None,
            )
            .expect("add game");

        let error = remove_stored_game(
            &store,
            LibraryGameId(game.id),
            Some(external.path()),
            Some(root.path()),
            true,
        )
        .expect_err("external prefix must be preserved");
        assert!(error.contains("Refusing to delete files outside"));
        assert_eq!(store.load().expect("game remains").len(), 1);
        assert!(external.path().exists());
    }

    #[test]
    fn library_move_stays_within_grid_boundaries() {
        assert_eq!(moved_index(3, 1, MoveDirection::Previous), Some(0));
        assert_eq!(moved_index(3, 1, MoveDirection::Next), Some(2));
        assert_eq!(moved_index(3, 0, MoveDirection::Previous), None);
        assert_eq!(moved_index(3, 2, MoveDirection::Next), None);
    }
}
