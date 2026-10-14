use crate::{Recognizer, Result};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Cpu,
    DirectMl,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Preference {
    Cpu,
    DirectMl,
    Auto,
}
impl Preference {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "cpu" => Ok(Self::Cpu),
            "dml" => Ok(Self::DirectMl),
            "auto" => Ok(Self::Auto),
            _ => Err("invalid-backend".into()),
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct BackendStatus {
    pub requested: Preference,
    pub active: Backend,
    pub fallback_count: u8,
    pub fallback_reason: Option<&'static str>,
}
impl BackendStatus {
    fn fallback(&mut self, reason: &'static str) -> bool {
        if self.requested != Preference::Auto
            || self.active != Backend::DirectMl
            || self.fallback_count != 0
        {
            return false;
        }
        self.active = Backend::Cpu;
        self.fallback_count = 1;
        self.fallback_reason = Some(reason);
        true
    }
}

// One native session at a time. A failed GPU session is dropped before CPU creation.
pub struct ResilientRecognizer {
    engine: Option<Recognizer>,
    model: PathBuf,
    dictionary: Vec<String>,
    catalog: HashMap<String, u32>,
    status: BackendStatus,
}
impl ResilientRecognizer {
    pub fn new(
        model: &Path,
        dictionary: Vec<String>,
        catalog: HashMap<String, u32>,
        preference: Preference,
    ) -> Result<Self> {
        let active = if preference == Preference::Cpu {
            Backend::Cpu
        } else {
            Backend::DirectMl
        };
        let mut status = BackendStatus {
            requested: preference,
            active,
            fallback_count: 0,
            fallback_reason: None,
        };
        let engine =
            match Recognizer::with_backend(model, dictionary.clone(), catalog.clone(), active) {
                Ok(engine) => engine,
                Err(error) => {
                    if !status.fallback("gpu_initialization_failed") {
                        return Err(error);
                    }
                    eprintln!("fuze-ocr: fallback=1");
                    Recognizer::with_backend(
                        model,
                        dictionary.clone(),
                        catalog.clone(),
                        Backend::Cpu,
                    )?
                }
            };
        Ok(Self {
            engine: Some(engine),
            model: model.into(),
            dictionary,
            catalog,
            status,
        })
    }
    pub fn status(&self) -> &BackendStatus {
        &self.status
    }
    pub fn warm_up(&mut self) -> Result<()> {
        self.run_inner("gpu_warmup_failed", false, |engine| engine.warm_up())
    }
    pub fn run<T>(
        &mut self,
        reason: &'static str,
        operation: impl FnMut(&mut Recognizer) -> Result<T>,
    ) -> Result<T> {
        self.run_inner(reason, true, operation)
    }
    fn run_inner<T>(
        &mut self,
        reason: &'static str,
        warm_replacement: bool,
        mut operation: impl FnMut(&mut Recognizer) -> Result<T>,
    ) -> Result<T> {
        let engine = self.engine.as_mut().ok_or("recognizer-failed")?;
        match operation(engine) {
            Ok(result) => Ok(result),
            Err(error) => {
                if !self.status.fallback(reason) {
                    return Err(error);
                }
                eprintln!("fuze-ocr: fallback=1");
                self.engine.take();
                let mut cpu = Recognizer::with_backend(
                    &self.model,
                    self.dictionary.clone(),
                    self.catalog.clone(),
                    Backend::Cpu,
                )?;
                if warm_replacement {
                    cpu.warm_up()?;
                }
                self.engine = Some(cpu);
                operation(self.engine.as_mut().unwrap())
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_is_one_way_and_explicit_modes_never_hide_failure() {
        for preference in [Preference::Cpu, Preference::DirectMl, Preference::Auto] {
            let mut status = BackendStatus {
                requested: preference,
                active: Backend::DirectMl,
                fallback_count: 0,
                fallback_reason: None,
            };
            assert_eq!(
                status.fallback("gpu_inference_failed"),
                preference == Preference::Auto
            );
            assert!(!status.fallback("gpu_inference_failed"));
        }
    }
}
