//! The terminal uses the same offline fonts as WebShell.
use nsh_core::{
    element::{AssetFile, AssetKind, Registry, Root},
    prelude::SharedString,
};

const TERMINAL_FAMILY: &str =
    "Hack Nerd Font Mono, Symbols Nerd Font Mono, Maple Mono NF CN, monospace";

pub(crate) fn terminal_family() -> SharedString {
    crate::preferences::current_context().font_family()
}

/// Resolve a saved primary family without reading context or reactive state.
pub(crate) fn resolve_terminal_family(family: &str) -> SharedString {
    use crate::preferences::font::{HACK, MAPLE, SYSTEM};

    match family {
        HACK => TERMINAL_FAMILY.into(),
        MAPLE => format!("{MAPLE}, {HACK}, Symbols Nerd Font Mono, {SYSTEM}").into(),
        SYSTEM => format!("{SYSTEM}, {HACK}, Symbols Nerd Font Mono, {MAPLE}").into(),
        family => format!("{family}, {TERMINAL_FAMILY}").into(),
    }
}

// Keep the source files shared with WebShell; do not vendor another copy here.
const FONT_ASSETS: &[AssetFile] = &[
    AssetFile {
        uri: "asset:/neko-remoted-client/fonts/HackNerdFontMono-Regular.ttf",
        source: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/frontend/public/fonts/preinstalled/HackNerdFontMono-Regular.ttf"
        ),
        kind: AssetKind::Font,
    },
    AssetFile {
        uri: "asset:/neko-remoted-client/fonts/SymbolsNerdFontMono-Regular.ttf",
        source: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/frontend/public/fonts/preinstalled/SymbolsNerdFontMono-Regular.ttf"
        ),
        kind: AssetKind::Font,
    },
    AssetFile {
        uri: "asset:/neko-remoted-client/fonts/MapleMono-NF-CN-Regular.ttf",
        source: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/frontend/public/fonts/preinstalled/MapleMono-NF-CN-Regular.ttf"
        ),
        kind: AssetKind::Font,
    },
    AssetFile {
        uri: "asset:/licenses/fonts/Hack-LICENSE.txt",
        source: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/frontend/public/fonts/licenses/Hack-LICENSE.txt"
        ),
        kind: AssetKind::File,
    },
    AssetFile {
        uri: "asset:/licenses/fonts/NerdFontsSymbolsOnly-LICENSE.txt",
        source: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/frontend/public/fonts/licenses/NerdFontsSymbolsOnly-LICENSE.txt"
        ),
        kind: AssetKind::File,
    },
    AssetFile {
        uri: "asset:/licenses/fonts/MapleMono-NF-CN-LICENSE.txt",
        source: concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../src/frontend/public/fonts/preinstalled/MapleMono-NF-CN-LICENSE.txt"
        ),
        kind: AssetKind::File,
    },
];

pub(crate) fn register(registry: &mut Registry) {
    // NSH's generated platform roots load these assets before measuring text.
    registry.assets(FONT_ASSETS);
}

pub(crate) fn root(root: Root) -> Root {
    // Android uses NSH's generated root and the registry assets. Embedding these
    // bytes there too would put a second copy of the fonts in the APK.
    #[cfg(not(target_os = "android"))]
    let root = root
        .font(
            &include_bytes!(
                "../../../src/frontend/public/fonts/preinstalled/HackNerdFontMono-Regular.ttf"
            )[..],
        )
        .font(
            &include_bytes!(
                "../../../src/frontend/public/fonts/preinstalled/SymbolsNerdFontMono-Regular.ttf"
            )[..],
        )
        .font(
            &include_bytes!(
                "../../../src/frontend/public/fonts/preinstalled/MapleMono-NF-CN-Regular.ttf"
            )[..],
        );
    root
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use nsh_core::prelude::*;

    #[component]
    fn FontProbe() -> impl IntoElement {
        text("font probe")
    }

    #[test]
    fn embedded_fonts_match_the_real_family_names_and_registry_assets() {
        let root = root(Root::of::<FontProbe>());
        let families: Vec<_> = root
            .fonts()
            .iter()
            .map(|data| nsh_core::font_family(data).unwrap())
            .collect();
        assert_eq!(
            families,
            [
                "Hack Nerd Font Mono",
                "Symbols Nerd Font Mono",
                "Maple Mono NF CN"
            ]
        );
        assert_eq!(
            resolve_terminal_family(crate::preferences::font::HACK).as_ref(),
            format!("{}, monospace", families.join(", "))
        );
        let mut registry = Registry::new();
        register(&mut registry);
        let assets: Vec<_> = registry.asset_files().collect();
        assert_eq!(assets.len(), 6);
        for (asset, bytes) in assets
            .iter()
            .filter(|asset| asset.kind == AssetKind::Font)
            .zip(root.fonts())
        {
            assert_eq!(asset.kind, AssetKind::Font);
            assert_eq!(std::fs::read(asset.source).unwrap(), bytes.as_ref());
        }
    }
}
