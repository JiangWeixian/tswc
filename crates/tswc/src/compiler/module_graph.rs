use log::debug;
use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use sugar_path::SugarPath;

use crate::config::Config;
use crate::resolver::Resolver;
use crate::utils::{QUERY_RE, SCRIPT_RE};

fn common_path_prefix(p1: &Path, p2: &Path) -> PathBuf {
  let mut common_prefix = PathBuf::new();
  let mut iter1 = p1.components();
  let mut iter2 = p2.components();

  while let (Some(c1), Some(c2)) = (iter1.next(), iter2.next()) {
    if c1 == c2 {
      common_prefix.push(c1.as_os_str());
    } else {
      break;
    }
  }

  common_prefix
}

fn replace_common_prefix(p1: &Path, p2: &Path, new_prefix: &Path) -> String {
  let common_prefix = common_path_prefix(p1, p2);
  let new_p1 = p1
    .strip_prefix(&common_prefix)
    .map(|suffix| new_prefix.join(suffix))
    .unwrap_or_else(|_| p1.to_path_buf());
  String::from(new_p1.to_str().unwrap_or_default())
}

fn clean_path(p: &str) -> String {
  let result = QUERY_RE.replace(p, "");
  return result.into();
}

fn get_matches(
  module: Option<&Module>,
  mg: &ModuleGraph,
  export_map: &mut HashMap<String, (String, String)>,
) {
  if let Some(m) = module {
    for (name, path, orig) in &m.export_map {
      export_map.insert(name.clone(), (path.clone(), orig.clone()));
    }
    if !m.export_wildcard.is_empty() {
      for specifier in &m.export_wildcard {
        let module = mg.get_module_by_specifier(specifier);
        // `if let some(...)` can prevent segmentation fault panic
        if let Some(module) = mg.modules.get(specifier) {
          get_matches(Some(module), mg, export_map);
        }
      }
    }
  }
}

#[derive(Default, Clone, Debug)]
pub struct Module {
  /// The imported named of the module
  pub specifier: String,
  /// The dir of current importee file
  pub context: String,
  /// is current module is compiled
  pub used: bool,
  /// Builtin node native modules
  pub built_in: bool,
  /// imported from node_modules
  pub is_node_modules: bool,
  /// is .jsx?|.tsx? file
  pub is_script: bool,
  /// Resolve failed
  pub not_found: bool,
  /// input files from options.include
  pub is_entry: bool,
  /// Resolved absolute filepath of specifier
  pub abs_path: String,
  /// Relative path relative to abs_path
  pub relative_path: String,
  /// Virtual absolute filepath, rewrite abs_path based on output.dir
  pub v_abs_path: String,
  /// Relative path relative to v_abs_path
  pub v_relative_path: String,
  /// is current module is optimized
  pub optimized: bool,
  /// see defines in barrel_visitor
  pub export_map: Vec<(String, String, String)>,
  /// see defines in barrel_visitor
  pub export_wildcard: Vec<String>,
  /// has export star
  pub is_wildcard: bool,
}

impl Module {
  // TODO: support custom ext
  pub fn with_ext(&self) -> Option<String> {
    if self.built_in || self.is_node_modules || self.not_found {
      return Some(self.specifier.clone());
    }
    if !self.is_script {
      return Some(self.v_relative_path.clone());
    }
    let path = self.v_relative_path.as_path().with_extension("js");
    path.to_str().map(|f| f.to_string())
  }
}

#[derive(Default, Debug)]
pub struct ModuleGraph {
  pub modules: HashMap<String, Module>,
  pub resolver: Resolver,
  pub config: Config,
}

impl ModuleGraph {
  pub fn new(resolver: Resolver, config: Config) -> ModuleGraph {
    Self {
      modules: Default::default(),
      resolver,
      config,
    }
  }
  pub fn add_module(&mut self, abs_path: &str, module: Module) -> Option<&mut Module> {
    if !self.modules.contains_key(abs_path) {
      self.modules.insert(abs_path.into(), module);
      self.modules.get_mut(abs_path)
    } else {
      self.modules.get_mut(abs_path)
    }
  }
  pub fn set_exports_info(
    &mut self,
    key: &str,
    export_map: Vec<(String, String, String)>,
    export_wildcards: Vec<String>,
  ) {
    let mut resolved_export_map = vec![];
    for (name, specifier, orig) in export_map {
      let module = self.resolve_esm_module(Some(specifier), key.to_string());
      if let Some(m) = module {
        resolved_export_map.push((name.clone(), m.abs_path.clone(), orig.clone()));
      }
    }
    let mut resolved_export_wildcards = vec![];
    // Create wildcard module
    for specifier in export_wildcards {
      let module = self.resolve_esm_module(Some(specifier), key.to_string());
      if let Some(m) = module {
        m.is_wildcard = true;
        resolved_export_wildcards.push(m.abs_path.clone());
      }
    }
    // TODO: maybe should create export_map instance for further get matches
    let module = self.modules.get_mut(key);
    if let Some(m) = module {
      m.export_map = resolved_export_map;
      m.export_wildcard = resolved_export_wildcards;
    }
  }
  // TODO: should save the result to self.export_map
  pub fn get_mappings(&self, specifier: &str) -> HashMap<String, (String, String)> {
    let mut export_map = HashMap::new();
    let module = self.get_module_by_specifier(specifier);
    get_matches(module, self, &mut export_map);
    return export_map;
  }
  pub fn resolve_entry_module(
    &mut self,
    specifier: Option<String>,
    is_wildcard: Option<bool>,
  ) -> Option<&mut Module> {
    if let Some(sp) = specifier {
      let abs_path = {
        let path = sp.as_path().absolutize();
        path.to_str().unwrap_or_default().to_string()
      };
      let v_abs_path = abs_path.replace(
        &self.config.resolved_options.input.to_str().unwrap(),
        &self.config.resolved_options.output.to_str().unwrap(),
      );
      let m = Module {
        specifier: sp,
        v_abs_path: String::from(v_abs_path),
        abs_path: String::from(&abs_path),
        is_entry: true,
        is_script: SCRIPT_RE.is_match(&abs_path),
        is_wildcard: is_wildcard.unwrap_or(false),
        ..Default::default()
      };
      self.add_module(&abs_path, m)
    } else {
      None
    }
  }
  /// Should merge resolve_esm_module & resolve_module into one
  pub fn resolve_esm_module(
    &mut self,
    specifier: Option<String>,
    context: String,
  ) -> Option<&mut Module> {
    // TODO: currently we resolve and add every module during compile
    // should we only resolve and add every module config in paths
    // TODO: should skip resolve if specifier and context found in module graph
    if let Some(sp) = specifier {
      let module = match self.resolver.resolve_module(&sp, &context) {
        Some(resolved) => {
          let abs_path: String = resolved
            .abs_path
            .and_then(|f| {
              // Webpack support add query on file suffix e.g. import svg from "path/icon.svg?url"
              // should clean path prevent unable to find real path on file system
              let p = clean_path(&f);
              return Some(p);
            })
            .unwrap_or("".into());
          let v_abs_path = replace_common_prefix(
            abs_path.as_path(),
            &self.config.resolved_options.input.as_path(),
            &self.config.resolved_options.output.as_path(),
          );
          let relative_path = resolved.relative_path;
          let context = resolved.context;
          let v_context = context.clone().and_then(|f| {
            Some(replace_common_prefix(
              &f.as_path(),
              &self.config.resolved_options.input.as_path(),
              &self.config.resolved_options.output.as_path(),
            ))
          });
          let v_relative_path = {
            let relative_path = v_abs_path
              .as_path()
              .relative(v_context.clone().unwrap_or_default().as_path());
            let relative_path = relative_path.to_str();
            relative_path.map(|f| {
              if f.starts_with(".") {
                f.to_string()
              } else {
                format!("./{}", f)
              }
            })
          };
          let is_script = SCRIPT_RE.is_match(&abs_path);
          debug!(
            target: "tswc",
            "abs_path {:?} v_abs_path {:?} is_script {:?}",
            abs_path, v_abs_path, is_script
          );
          let m = Module {
            specifier: sp,
            context: context.unwrap_or_default(),
            is_script,
            abs_path: abs_path.clone(),
            v_abs_path: v_abs_path.into(),
            relative_path: relative_path.unwrap_or_default(),
            v_relative_path: v_relative_path.unwrap_or_default(),
            // TODO: maybe renamed to skip compile
            used: resolved.built_in || resolved.is_node_modules || resolved.not_found,
            is_node_modules: resolved.is_node_modules,
            not_found: resolved.not_found,
            built_in: resolved.built_in,
            ..Default::default()
          };
          // FIXME: if abs_path releated is already inserted; self.add_module take no effect
          // modify m after resolve_module will not working on self.modules[abs_path]
          // and cloned module here, it mean m !== self.modules[abs_path]
          self.add_module(&abs_path, m)
        }
        None => None,
      };
      module
    } else {
      None
    }
  }
  /// Also add resolved module into self.modules
  pub fn resolve_module(
    &mut self,
    specifier: Option<String>,
    context: String,
  ) -> Option<&mut Module> {
    // TODO: currently we resolve and add every module during compile
    // should we only resolve and add every module config in paths
    // TODO: should skip resolve if specifier and context found in module graph
    if let Some(sp) = specifier {
      let module = match self.resolver.resolve(&sp, &context) {
        Some(resolved) => {
          let abs_path: String = resolved
            .abs_path
            .and_then(|f| {
              // Webpack support add query on file suffix e.g. import svg from "path/icon.svg?url"
              // should clean path prevent unable to find real path on file system
              let p = clean_path(&f);
              return Some(p);
            })
            .unwrap_or("".into());
          let v_abs_path = replace_common_prefix(
            abs_path.as_path(),
            &self.config.resolved_options.input.as_path(),
            &self.config.resolved_options.output.as_path(),
          );
          let relative_path = resolved.relative_path;
          let context = resolved.context;
          let v_context = context.clone().and_then(|f| {
            Some(replace_common_prefix(
              &f.as_path(),
              &self.config.resolved_options.input.as_path(),
              &self.config.resolved_options.output.as_path(),
            ))
          });
          let v_relative_path = {
            let relative_path = v_abs_path
              .as_path()
              .relative(v_context.clone().unwrap_or_default().as_path());
            let relative_path = relative_path.to_str();
            relative_path.map(|f| {
              if f.starts_with(".") {
                f.to_string()
              } else {
                format!("./{}", f)
              }
            })
          };
          let is_script = SCRIPT_RE.is_match(&abs_path);
          debug!(
            target: "tswc",
            "abs_path {:?} v_abs_path {:?} is_script {:?}",
            abs_path, v_abs_path, is_script
          );
          let m = Module {
            specifier: sp,
            context: context.unwrap_or_default(),
            is_script,
            abs_path: abs_path.clone(),
            v_abs_path: v_abs_path.into(),
            relative_path: relative_path.unwrap_or_default(),
            v_relative_path: v_relative_path.unwrap_or_default(),
            // TODO: maybe renamed to skip compile
            used: resolved.built_in || resolved.is_node_modules || resolved.not_found,
            is_node_modules: resolved.is_node_modules,
            not_found: resolved.not_found,
            built_in: resolved.built_in,
            ..Default::default()
          };
          // FIXME: if abs_path releated is already inserted; self.add_module take no effect
          // modify m after resolve_module will not working on self.modules[abs_path]
          // and cloned module here, it mean m !== self.modules[abs_path]
          self.add_module(&abs_path, m)
        }
        None => None,
      };
      module
    } else {
      None
    }
  }
  pub fn get_module_by_specifier(&self, specifier: &str) -> Option<&Module> {
    self.modules.values().find(|m| m.specifier == specifier)
  }
  pub fn get_unused_modules(&mut self) -> impl Iterator<Item = &mut Module> {
    self.modules.values_mut().filter(|module| !module.used)
  }
  pub fn get_unused_modules_size(&self) -> usize {
    let modules: Vec<&Module> = self
      .modules
      .values()
      .into_iter()
      .filter(|f| !f.used)
      .collect();
    modules.len()
  }
  pub fn get_wildcard_modules_size(&self) -> usize {
    let modules: Vec<&Module> = self
      .modules
      .values()
      .into_iter()
      .filter(|f| f.is_wildcard && !f.optimized)
      .collect();
    modules.len()
  }
  pub fn get_wildcard_modules(&mut self) -> impl Iterator<Item = &mut Module> {
    self
      .modules
      .values_mut()
      .filter(|module| module.is_wildcard && !module.optimized)
  }
}
