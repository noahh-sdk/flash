use clang::{Accessibility, Entity, EntityKind};
use serde_json::json;

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use tokio::task::JoinHandle;

use crate::{
    config::{Config, ExternalLib, Source},
    html::Html,
    url::UrlPath,
};

use super::{builder::Builder, namespace::CppItemKind};

pub trait EntityMethods<'e> {
    /// Get the config source for this entity
    fn config_source(&self, config: Arc<Config>) -> Option<Arc<Source>>;

    /// Get the file where this entity is defined, if applicable
    fn definition_file(&self) -> Option<PathBuf>;

    /// Get a relative path to this file's header, if applicable
    fn header(&self, config: Arc<Config>) -> Option<PathBuf>;

    /// Get the relative for this entity
    fn rel_docs_url(&self) -> Option<UrlPath>;

    /// Get the full URL for this entity, valid for links
    fn abs_docs_url(&self, config: Arc<Config>) -> Option<UrlPath>;

    /// Get the full online URL of this entity
    fn github_url(&self, config: Arc<Config>) -> Option<String>;

    /// Get the include path for this entity
    fn include_path(&self, config: Arc<Config>) -> Option<UrlPath>;

    /// Get the fully qualified name for this entity
    fn full_name(&self) -> Vec<String>;

    /// Get the parents of this entity
    fn ancestorage(&self) -> Vec<Entity<'e>>;

    /// Gets all the member functions from this entity, assuming it is a class-like entity
    fn get_member_functions(&self, visibility: Access, include_statics: Include)
        -> Vec<Entity<'e>>;

    /// Gets the function arguments for this method, including templated ones
    fn get_function_arguments(&self) -> Option<Vec<Entity<'e>>>;

    /// Extracts the source code that makes up this entity, according to its range.
    /// This might have to read a file from disk, and also includes whitespace and all
    fn extract_source_string(&self) -> Option<String>;

    /// Same as extract_source_string, but removes new lines, double spaces, and leading/trailing whitespace
    fn extract_source_string_cleaned(&self) -> Option<String>;

    /// Checks if the entitiy is in one of the allowed external libraries
    fn get_allowed_external_lib(&self, config: Arc<Config>) -> Option<Arc<ExternalLib>>;
}

impl<'e> EntityMethods<'e> for Entity<'e> {
    fn config_source(&self, config: Arc<Config>) -> Option<Arc<Source>> {
        // Get the definition header
        let path = self.header(config.clone())?;

        // Find the source that has this header
        config
            .sources
            .iter()
            .find(|src| path.starts_with(src.dir.to_pathbuf()))
            .cloned()
    }

    fn definition_file(&self) -> Option<PathBuf> {
        self.get_definition()
            .map_or_else(|| self.get_location(), |d| d.get_location())?
            .get_file_location()
            .file?
            .get_path()
            .into()
    }

    fn header(&self, config: Arc<Config>) -> Option<PathBuf> {
        let path = self.definition_file()?;
        path.strip_prefix(&config.input_dir)
            .unwrap_or(&path)
            .to_path_buf()
            .into()
    }

    fn rel_docs_url(&self) -> Option<UrlPath> {
        Some(
            CppItemKind::from(self)?
                .docs_category()
                .join(UrlPath::new_with_path(self.full_name())),
        )
    }

    fn abs_docs_url(&self, config: Arc<Config>) -> Option<UrlPath> {
        // If this is an std item, redirect to cppreference instead
        if self.full_name().first().is_some_and(|n| n == "std") {
            UrlPath::parse(&format!(
                "en.cppreference.com/w/cpp/{}/{}",
                self.definition_file()?.file_name()?.to_str()?,
                self.get_name()?
            ))
            .ok()
        } else {
            Some(self.rel_docs_url()?.to_absolute(config))
        }
    }

    fn github_url(&self, config: Arc<Config>) -> Option<String> {
        if self.full_name().first().is_some_and(|n| n == "std") {
            unreachable!(
                "Shouldn't be trying to link to a stl header - {:?}",
                self.get_name()
            )
        } else if let Some(lib) = self.get_allowed_external_lib(config.clone()) {
            Some(lib.repository.clone())
        } else {
            Some(
                config.project.tree.clone()?
                    + UrlPath::try_from(&self.header(config)?)
                        .ok()?
                        .to_string()
                        .as_str(),
            )
        }
    }

    fn include_path(&self, config: Arc<Config>) -> Option<UrlPath> {
        if self.is_in_system_header() {
            return UrlPath::part(
                self.header(config.clone())?
                    .file_name()?
                    .to_string_lossy()
                    .as_ref(),
            )
            .into();
        }
        UrlPath::try_from(&self.header(config.clone())?)
            .ok()?
            .strip_prefix(&self.config_source(config)?.dir)
            .into()
    }

    fn full_name(&self) -> Vec<String> {
        self.ancestorage()
            .iter()
            .map(|a| a.get_name().unwrap_or("_anon".into()))
            .collect()
    }

    fn ancestorage(&self) -> Vec<Entity<'e>> {
        let mut ancestors = Vec::new();
        if let Some(parent) = self.get_semantic_parent() {
            // apparently in github actions TranslationUnit enum doesn't
            // match, so use this as a fail-safe
            if !parent.get_name().is_some_and(|p| p.ends_with(".cpp")) {
                match parent.get_kind() {
                    EntityKind::TranslationUnit
                    | EntityKind::UnexposedDecl
                    | EntityKind::UnexposedAttr
                    | EntityKind::UnexposedExpr
                    | EntityKind::UnexposedStmt => {}
                    _ => ancestors.extend(parent.ancestorage()),
                }
            }
        }
        // remove leading anon namespaces
        let mut ancestors: Vec<_> = ancestors
            .into_iter()
            .skip_while(|e| e.get_name().is_none())
            .collect();
        ancestors.push(*self);
        ancestors
    }

    fn get_member_functions(
        &self,
        visibility: Access,
        include_statics: Include,
    ) -> Vec<Entity<'e>> {
        self.get_children()
            .into_iter()
            .filter(|child| {
                (child.get_kind() == EntityKind::Method
                    || child.get_kind() == EntityKind::FunctionTemplate)
                    && match include_statics {
                        Include::Members => !child.is_static_method(),
                        Include::Statics => child.is_static_method(),
                        Include::All => true,
                    }
                    && match child.get_accessibility() {
                        Some(Accessibility::Protected) => {
                            matches!(visibility, Access::All | Access::Protected)
                        }
                        Some(Accessibility::Public) => {
                            matches!(visibility, Access::All | Access::Public)
                        }
                        _ => false,
                    }
            })
            .collect()
    }

    fn get_function_arguments(&self) -> Option<Vec<Entity<'e>>> {
        if !matches!(
            self.get_kind(),
            EntityKind::FunctionTemplate | EntityKind::FunctionDecl | EntityKind::Method
        ) {
            return None;
        }
        let mut args = vec![];
        self.visit_children(|child, _| {
            if child.get_kind() == EntityKind::ParmDecl {
                args.push(child);
            }
            clang::EntityVisitResult::Continue
        });
        Some(args)
    }

    fn extract_source_string(&self) -> Option<String> {
        let range = self.get_range()?;
        let start = range.get_start().get_file_location();
        let end = range.get_end().get_file_location();
        let contents = start.file?.get_contents()?;
        contents
            .get(start.offset as usize..end.offset as usize)
            .map(|s| s.into())
    }

    fn extract_source_string_cleaned(&self) -> Option<String> {
        // TODO: this code is stupid and should be improved
        Some(
            self.extract_source_string()?
                .trim()
                .replace('\t', " ")
                .replace(['\r', '\n'], "")
                .split(' ')
                .filter(|x| !x.is_empty())
                .intersperse(" ")
                .collect(),
        )
    }

    fn get_allowed_external_lib(&self, config: Arc<Config>) -> Option<Arc<ExternalLib>> {
        self.is_in_system_header()
            .then(|| self.get_location())
            .flatten()
            .and_then(|x| x.get_file_location().file)
            .and_then(|file| {
                let path = file.get_path();
                let path = path.to_string_lossy();
                config
                    .external_libs
                    .iter()
                    .find(|f| path.contains(&f.pattern))
            })
            .cloned()
    }
}

#[derive(Clone)]
pub struct SubItem {
    pub title: String,
}

impl SubItem {
    pub fn for_classlike(entity: &Entity) -> Vec<SubItem> {
        let Some(kind) = CppItemKind::from(entity) else {
            return Vec::new();
        };
        match kind {
            CppItemKind::Class | CppItemKind::Struct => entity
                .get_member_functions(Access::All, Include::All)
                .into_iter()
                .filter_map(|e| {
                    Some(SubItem {
                        title: e.get_name()?,
                    })
                })
                .collect(),

            CppItemKind::Namespace | CppItemKind::Function => Vec::new(),
        }
    }
}

pub enum NavItem {
    Root(Option<String>, Vec<NavItem>),
    Dir(String, Vec<NavItem>, Option<(String, bool)>, bool),
    Link(String, UrlPath, Option<(String, bool)>, Vec<SubItem>),
}

impl NavItem {
    pub fn new_link(
        name: &str,
        url: UrlPath,
        icon: Option<(&str, bool)>,
        suboptions: Vec<SubItem>,
    ) -> NavItem {
        NavItem::Link(
            name.into(),
            url,
            icon.map(|s| (s.0.into(), s.1)),
            suboptions,
        )
    }

    pub fn new_dir(name: &str, items: Vec<NavItem>, icon: Option<(&str, bool)>) -> NavItem {
        NavItem::Dir(name.into(), items, icon.map(|s| (s.0.into(), s.1)), false)
    }

    pub fn new_dir_open(
        name: &str,
        items: Vec<NavItem>,
        icon: Option<(&str, bool)>,
        open: bool,
    ) -> NavItem {
        NavItem::Dir(name.into(), items, icon.map(|s| (s.0.into(), s.1)), open)
    }

    pub fn new_root(name: Option<&str>, items: Vec<NavItem>) -> NavItem {
        NavItem::Root(name.map(|s| s.into()), items)
    }

    pub fn suboptions_titles(&self, config: Arc<Config>) -> HashMap<String, usize> {
        match self {
            NavItem::Link(name, _, _, suboptions) => {
                let mut res = HashMap::new();
                for opt in suboptions.iter().map(|o| format!("{}::{}", name, o.title)) {
                    if let Some(r) = res.get_mut(&opt) {
                        *r += 1;
                    } else {
                        res.insert(opt, 0);
                    }
                }
                res
            }

            NavItem::Dir(name, items, _, _) => items
                .iter()
                .flat_map(|i| i.suboptions_titles(config.clone()))
                .map(|(t, count)| (format!("{}::{}", name, t), count))
                .collect(),

            NavItem::Root(_, items) => items
                .iter()
                .flat_map(|i| i.suboptions_titles(config.clone()))
                .collect(),
        }
    }

    pub fn to_json(&self, config: Arc<Config>) -> serde_json::Value {
        match self {
            NavItem::Link(name, url, icon, _) => {
                json!({
                    "type": "link",
                    "icon": icon,
                    "name": name,
                    "url": url.to_absolute(config.clone()).to_string(),
                })
            }

            NavItem::Dir(name, items, icon, open) => {
                json!({
                    "type": "dir",
                    "icon": icon,
                    "name": name,
                    "open": open,
                    "items": items.iter().map(|x| x.to_json(config.clone())).collect::<Vec<_>>()
                })
            }

            NavItem::Root(name, items) => {
                json!({
                    "type": "root",
                    "name": name,
                    "items": items.iter().map(|x| x.to_json(config.clone())).collect::<Vec<_>>()
                })
            }
        }
    }
}

pub type BuildResult = Result<Vec<JoinHandle<Result<UrlPath, String>>>, String>;

pub trait Entry<'e> {
    fn name(&self) -> String;
    fn url(&self) -> UrlPath;
    fn build(&self, builder: &Builder<'e>) -> BuildResult;
    fn nav(&self) -> NavItem;
}

pub trait OutputEntry<'e>: Entry<'e> {
    fn output(&self, builder: &'e Builder<'e>) -> (Arc<String>, Vec<(&'static str, Html)>);
    fn description(&self, builder: &'e Builder<'e>) -> String;
}

pub trait ASTEntry<'e>: Entry<'e> {
    fn entity(&self) -> &Entity<'e>;
    fn category(&self) -> &'static str;
    fn output_description(&self, builder: &'e Builder<'e>) -> String {
        format!(
            "Documentation for the {} {} in {}",
            self.name(),
            self.category(),
            builder.config.project.name
        )
    }
}

pub enum Access {
    All,
    Public,
    Protected,
}

pub enum Include {
    All,
    Members,
    Statics,
}
