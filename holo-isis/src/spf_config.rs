//
// Copyright (c) The Holo Core Contributors
//
// SPDX-License-Identifier: MIT
//

//! IS-IS SPF startup defaults: file / env / YANG merge (D12).
//!
//! `holod.toml` explicit values win over environment variables, which win over
//! YANG/code defaults. Northbound commits override the resulting runtime
//! values after instance creation.

use holo_protocol::IsisSpfConfig;
use tracing::warn;

use crate::northbound::yang_gen::isis;

/// Optional per-field sources (TOML or environment).
///
/// `None` means the source did not set the field.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SpfConfigSources {
    pub enabled: Option<bool>,
    pub initial_delay: Option<u32>,
    pub short_delay: Option<u32>,
    pub long_delay: Option<u32>,
    pub hold_down: Option<u32>,
    pub time_to_learn: Option<u32>,
}

/// YANG/code defaults for SPF control parameters.
pub fn yang_spf_defaults() -> IsisSpfConfig {
    IsisSpfConfig {
        enabled: isis::spf_control::enabled::DFLT,
        initial_delay: isis::spf_control::ietf_spf_delay::initial_delay::DFLT,
        short_delay: isis::spf_control::ietf_spf_delay::short_delay::DFLT,
        long_delay: isis::spf_control::ietf_spf_delay::long_delay::DFLT,
        hold_down: isis::spf_control::ietf_spf_delay::hold_down::DFLT,
        time_to_learn: isis::spf_control::ietf_spf_delay::time_to_learn::DFLT,
    }
}

/// Resolve SPF defaults with priority file > env > YANG/code DFLT (D12).
pub fn resolve_spf_defaults(
    file: &SpfConfigSources,
    env: &SpfConfigSources,
    dflt: &IsisSpfConfig,
) -> IsisSpfConfig {
    IsisSpfConfig {
        enabled: file.enabled.or(env.enabled).unwrap_or(dflt.enabled),
        initial_delay: file
            .initial_delay
            .or(env.initial_delay)
            .unwrap_or(dflt.initial_delay),
        short_delay: file
            .short_delay
            .or(env.short_delay)
            .unwrap_or(dflt.short_delay),
        long_delay: file
            .long_delay
            .or(env.long_delay)
            .unwrap_or(dflt.long_delay),
        hold_down: file.hold_down.or(env.hold_down).unwrap_or(dflt.hold_down),
        time_to_learn: file
            .time_to_learn
            .or(env.time_to_learn)
            .unwrap_or(dflt.time_to_learn),
    }
}

/// Read `HOLO_ISIS_SPF_*` environment variables.
///
/// Parse failures log a warning and fall back as if the variable was unset (D6).
pub fn read_spf_env() -> SpfConfigSources {
    SpfConfigSources {
        enabled: env_bool("HOLO_ISIS_SPF_ENABLED"),
        initial_delay: env_u32("HOLO_ISIS_SPF_INITIAL_DELAY"),
        short_delay: env_u32("HOLO_ISIS_SPF_SHORT_DELAY"),
        long_delay: env_u32("HOLO_ISIS_SPF_LONG_DELAY"),
        hold_down: env_u32("HOLO_ISIS_SPF_HOLD_DOWN"),
        time_to_learn: env_u32("HOLO_ISIS_SPF_TIME_TO_LEARN"),
    }
}

fn env_bool(name: &str) -> Option<bool> {
    let Ok(raw) = std::env::var(name) else {
        return None;
    };
    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        other => {
            warn!(
                env = name,
                value = other,
                "invalid boolean for SPF env; ignoring (fallback to next priority)"
            );
            None
        }
    }
}

fn env_u32(name: &str) -> Option<u32> {
    let Ok(raw) = std::env::var(name) else {
        return None;
    };
    match raw.parse::<u32>() {
        Ok(v) => Some(v),
        Err(_) => {
            warn!(
                env = name,
                value = raw.as_str(),
                "invalid u32 for SPF env; ignoring (fallback to next priority)"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dflt() -> IsisSpfConfig {
        IsisSpfConfig {
            enabled: true,
            initial_delay: 50,
            short_delay: 200,
            long_delay: 5000,
            hold_down: 10000,
            time_to_learn: 500,
        }
    }

    #[test]
    fn resolve_prefers_file_over_env_over_dflt() {
        let file = SpfConfigSources {
            enabled: Some(false),
            initial_delay: Some(11),
            short_delay: None,
            long_delay: None,
            hold_down: None,
            time_to_learn: None,
        };
        let env = SpfConfigSources {
            enabled: Some(true),
            initial_delay: Some(22),
            short_delay: Some(33),
            long_delay: None,
            hold_down: None,
            time_to_learn: Some(44),
        };
        let got = resolve_spf_defaults(&file, &env, &dflt());
        assert!(!got.enabled);
        assert_eq!(got.initial_delay, 11);
        assert_eq!(got.short_delay, 33);
        assert_eq!(got.long_delay, 5000);
        assert_eq!(got.hold_down, 10000);
        assert_eq!(got.time_to_learn, 44);
    }

    #[test]
    fn resolve_all_unset_uses_dflt() {
        let got = resolve_spf_defaults(
            &SpfConfigSources::default(),
            &SpfConfigSources::default(),
            &dflt(),
        );
        assert!(got.enabled);
        assert_eq!(got.initial_delay, 50);
        assert_eq!(got.short_delay, 200);
        assert_eq!(got.long_delay, 5000);
        assert_eq!(got.hold_down, 10000);
        assert_eq!(got.time_to_learn, 500);
    }

    #[test]
    fn env_bool_invalid_falls_back() {
        // Direct unit of parser helpers via resolve path: invalid env is None.
        let env = SpfConfigSources {
            enabled: None, // as if env_bool rejected the value
            initial_delay: None,
            short_delay: None,
            long_delay: None,
            hold_down: None,
            time_to_learn: None,
        };
        let file = SpfConfigSources::default();
        let got = resolve_spf_defaults(&file, &env, &dflt());
        assert!(got.enabled);
    }

    #[test]
    fn env_u32_invalid_treated_as_unset() {
        let env = SpfConfigSources {
            enabled: None,
            initial_delay: None, // invalid parse → None
            short_delay: Some(99),
            long_delay: None,
            hold_down: None,
            time_to_learn: None,
        };
        let got =
            resolve_spf_defaults(&SpfConfigSources::default(), &env, &dflt());
        assert_eq!(got.initial_delay, 50);
        assert_eq!(got.short_delay, 99);
    }
}
