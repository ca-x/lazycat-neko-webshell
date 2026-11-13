//! One branded icon for the app and system tray, embedded without file lookups.
use nsh_core::element::ImageSrc;
use std::sync::LazyLock;

pub(super) fn source() -> ImageSrc {
    static URI: LazyLock<String> = LazyLock::new(|| {
        use std::fmt::Write;
        let mut uri = String::from("data:image/png,");
        for byte in include_bytes!("../../../android/res/drawable/neko.png") {
            write!(uri, "%{byte:02X}").expect("writing to a String");
        }
        uri
    });
    ImageSrc::from(URI.as_str())
}
