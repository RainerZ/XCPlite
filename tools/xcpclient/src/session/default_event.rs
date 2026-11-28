//-----------------------------------------------------------------------------
// Module session::default_event
// The event for variables without a fixed event (--default-event), given by event id or event name

/// Value of --default-event: an event id or an event name.
/// A name follows the C identifier rules and is resolved to an event id against the registry event list once it is complete.
#[derive(Debug, Clone, PartialEq)]
pub enum DefaultEvent {
    Id(u16),
    Name(String),
}

impl DefaultEvent {
    /// Resolve to an event id
    /// An id is taken as it is, a name is looked up in the registry event list and is an error when not found
    pub fn resolve(&self, reg: &xcp_registry::Registry) -> Result<u16, String> {
        match self {
            DefaultEvent::Id(id) => Ok(*id),
            DefaultEvent::Name(name) => reg
                .event_list
                .iter()
                .find(|e| e.get_unique_name(reg) == name.as_str() || (e.index == 0 && e.get_name() == name.as_str()))
                .map(|e| e.get_id())
                .ok_or_else(|| {
                    let known: Vec<String> = reg.event_list.iter().map(|e| format!("{} ({})", e.get_unique_name(reg), e.get_id())).collect();
                    format!("Default event '{}' not found in the event list [{}]", name, known.join(", "))
                }),
        }
    }
}

impl std::fmt::Display for DefaultEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DefaultEvent::Id(id) => write!(f, "{}", id),
            DefaultEvent::Name(name) => write!(f, "{}", name),
        }
    }
}

impl std::str::FromStr for DefaultEvent {
    type Err = String;

    /// An unsigned integer is an event id, a C identifier is an event name, anything else is an error
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if let Ok(id) = s.parse::<u16>() {
            return Ok(DefaultEvent::Id(id));
        }
        let mut chars = s.chars();
        let is_identifier = matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic()) && chars.all(|c| c == '_' || c.is_ascii_alphanumeric());
        if is_identifier {
            Ok(DefaultEvent::Name(s.to_string()))
        } else {
            Err(format!("'{}' is neither an event id (0..65535) nor an event name (C identifier)", s))
        }
    }
}

/// Config file: accept an integer (event id) or a string (event id or event name)
impl<'de> serde::Deserialize<'de> for DefaultEvent {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Id(u16),
            Text(String),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Id(id) => Ok(DefaultEvent::Id(id)),
            Raw::Text(s) => s.parse().map_err(serde::de::Error::custom),
        }
    }
}

//------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use figment::{
        Figment,
        providers::{Format, Toml},
    };
    use xcp_registry::McEvent;

    #[test]
    fn test_default_event_parse() {
        assert_eq!("3".parse::<DefaultEvent>(), Ok(DefaultEvent::Id(3)));
        assert_eq!(" 65535 ".parse::<DefaultEvent>(), Ok(DefaultEvent::Id(65535)));
        assert_eq!("mainloop".parse::<DefaultEvent>(), Ok(DefaultEvent::Name("mainloop".into())));
        assert_eq!("_task_1".parse::<DefaultEvent>(), Ok(DefaultEvent::Name("_task_1".into())));
        assert!("65536".parse::<DefaultEvent>().is_err()); // not a u16, not an identifier
        assert!("-1".parse::<DefaultEvent>().is_err());
        assert!("1abc".parse::<DefaultEvent>().is_err());
        assert!("main loop".parse::<DefaultEvent>().is_err());
        assert!("main-loop".parse::<DefaultEvent>().is_err());
        assert!("".parse::<DefaultEvent>().is_err());
    }

    #[test]
    fn test_default_event_resolve() {
        let mut reg = xcp_registry::Registry::new();
        reg.event_list.add_event(McEvent::new("mainloop", 0, 7, 0)).unwrap();
        reg.event_list.add_event(McEvent::new("task", 1, 8, 0)).unwrap();
        reg.event_list.add_event(McEvent::new("task", 2, 9, 0)).unwrap();

        assert_eq!(DefaultEvent::Id(42).resolve(&reg), Ok(42)); // ids are not checked here
        assert_eq!(DefaultEvent::Name("mainloop".into()).resolve(&reg), Ok(7));
        assert_eq!(DefaultEvent::Name("task_2".into()).resolve(&reg), Ok(9)); // multi instance event by unique name
        assert!(DefaultEvent::Name("task".into()).resolve(&reg).is_err()); // ambiguous, only exists with index > 0
        assert!(DefaultEvent::Name("unknown".into()).resolve(&reg).is_err());
    }

    #[test]
    fn test_default_event_config_file() {
        #[derive(serde::Deserialize, Default)]
        #[serde(default)]
        struct Cfg {
            default_event: Option<DefaultEvent>,
        }
        let parse = |text: &str| Figment::new().merge(Toml::string(text)).extract::<Cfg>().map_err(|e| e.to_string());
        assert_eq!(parse("default_event = 3").unwrap().default_event, Some(DefaultEvent::Id(3)));
        assert_eq!(parse("default_event = \"3\"").unwrap().default_event, Some(DefaultEvent::Id(3)));
        assert_eq!(parse("default_event = \"mainloop\"").unwrap().default_event, Some(DefaultEvent::Name("mainloop".into())));
        assert!(parse("default_event = \"main loop\"").is_err());
        assert_eq!(parse("").unwrap().default_event, None);
    }
}
