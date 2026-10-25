pub(crate) const CATFLAP_APPS: [&str; 2] = [
    "community.lazycat.app.catflap",
    "community.lazycat.app.catflap.dev",
];
pub(crate) const CATFLAP_PROTOCOL: &str = "catflap.client-terminal.v1";
pub(crate) const CATFLAP_CREDENTIAL_HEADER: &str = "x-catflap-terminal-ticket";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClientBroker {
    LightOs,
    Catflap(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClientTarget<'a> {
    pub broker: ClientBroker,
    pub id: &'a str,
}

impl<'a> ClientTarget<'a> {
    pub fn parse(selector: &'a str) -> Option<Self> {
        let value = selector.trim().strip_prefix("client:")?;
        let (broker, id) = if let Some(value) = value.strip_prefix("catflap:") {
            let (app, id) = value.split_once(':')?;
            let app = *CATFLAP_APPS.iter().find(|candidate| **candidate == app)?;
            if !valid_catflap_id(id) {
                return None;
            }
            (ClientBroker::Catflap(app), id)
        } else {
            (ClientBroker::LightOs, value)
        };
        if id.is_empty()
            || id.len() > 256
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return None;
        }
        Some(Self { broker, id })
    }
}

pub(crate) fn valid_catflap_id(id: &str) -> bool {
    id.strip_prefix("pc-").is_some_and(|suffix| {
        suffix.len() == 32
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

impl ClientBroker {
    pub fn selector(self, id: &str) -> String {
        match self {
            Self::LightOs => format!("client:{id}"),
            Self::Catflap(app) => format!("client:catflap:{app}:{id}"),
        }
    }

    pub fn app(self) -> Option<&'static str> {
        match self {
            Self::LightOs => None,
            Self::Catflap(app) => Some(app),
        }
    }

    pub fn from_origin(origin: &str) -> Option<Self> {
        CATFLAP_APPS.iter().find_map(|app| {
            (crate::catflap_terminal::origin(app).ok() == Some(origin))
                .then_some(Self::Catflap(app))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_brokers_and_rejects_unknown_origins_or_registration_ids() {
        let id = "pc-0123456789abcdef0123456789abcdef";
        let official = ClientBroker::LightOs.selector(id);
        let production = ClientBroker::Catflap(CATFLAP_APPS[0]).selector(id);
        let development = ClientBroker::Catflap(CATFLAP_APPS[1]).selector(id);
        assert_ne!(official, production);
        assert_ne!(production, development);
        assert_eq!(ClientTarget::parse(&production).unwrap().id, id);
        for invalid in [
            "client:catflap:evil.example:pc-0123456789abcdef0123456789abcdef",
            "client:catflap:community.lazycat.app.catflap:device-a",
            "client:catflap:community.lazycat.app.catflap:pc-0123456789ABCDEF0123456789abcdef",
            "client:catflap:community.lazycat.app.catflap:../escape",
        ] {
            assert!(ClientTarget::parse(invalid).is_none());
        }
        assert!(ClientBroker::from_origin("https://evil.example").is_none());
    }
}
