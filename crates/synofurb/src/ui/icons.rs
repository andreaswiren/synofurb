//! Icons: Lucide, through `gpui-kit-assets` (`IconName`), and nothing else. No emoji, no hand-drawn SVG (the logo
//! lives in [`super::brand`]).
//!
//! Screens never name a Lucide icon directly; they name a *role* (`icons::role::NEXT`). One macro below is the single
//! source of truth: it generates the role constants AND the embedded asset source, so an icon that is used is always
//! bundled (only these SVGs are compiled in, not the 1,830 of the full catalogue).

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{Hsla, px};

macro_rules! roles {
    ($($(#[$m:meta])* $role:ident => $icon:ident,)*) => {
        /// Semantic role -> Lucide icon.
        #[allow(dead_code)] // the whole vocabulary is kept so a design revision can use any role
        pub mod role {
            use gpui_kit::assets::IconName;
            $($(#[$m])* pub const $role: IconName = IconName::$icon;)*
        }

        /// Every role with its constant name, for tests and the component documentation.
        #[allow(dead_code)]
        pub const ALL: &[(&str, IconName)] = &[$((stringify!($role), IconName::$icon),)*];

        gpui_kit::assets::icon_assets!(pub RoleIcons, [$($icon),*]);
    };
}

roles! {
    // wizard steps and side pages
    STEP_FIND => Radar,
    STEP_CONNECT => Plug,
    STEP_INSPECT => ScanSearch,
    STEP_GOAL => Flag,
    STEP_STORAGE => HardDrive,
    STEP_SYSTEM => Cpu,
    STEP_CONFIRM => ShieldCheck,
    STEP_INSTALL => Download,
    STEP_DONE => CircleCheck,
    SETTINGS => Settings,
    HELP => LifeBuoy,
    ABOUT => Info,
    BOOT_MANAGER => Usb,
    RECOVER => Undo2,
    MANAGE => ExternalLink,
    // status (always paired with a word)
    OK => CircleCheck,
    WARN => TriangleAlert,
    ERR => CircleX,
    INFO => Info,
    PENDING => CircleDashed,
    RUNNING => LoaderCircle,
    CURRENT => CircleDot,
    STOP => OctagonX,
    SAFE => ShieldCheck,
    DANGER => ShieldAlert,
    // compatibility list
    COMPAT_VERIFIED => ShieldCheck,
    COMPAT_COMMUNITY => Users,
    COMPAT_UNTESTED => CircleQuestionMark,
    COMPAT_ISSUES => TriangleAlert,
    COMPAT_UNSUPPORTED => Ban,
    // actions
    BACK => ArrowLeft,
    NEXT => ArrowRight,
    COPY => Copy,
    DOWNLOAD => Download,
    UPLOAD => Upload,
    REFRESH => RefreshCw,
    SEARCH => Search,
    CLOSE => X,
    FOLDER => FolderOpen,
    EXTERNAL => ExternalLink,
    SAVE => Save,
    SHOW => Eye,
    HIDE => EyeOff,
    DELETE => Trash,
    PLAY => Play,
    CHECK => Check,
    EDIT => SquarePen,
    ROTATE_BACK => RotateCcw,
    EXPAND => ChevronDown,
    COLLAPSE => ChevronRight,
    // contextual help
    TIP => CircleQuestionMark,
    KEYBOARD => Keyboard,
    BOOK => BookOpen,
    LINK => Link,
    // things
    DISK => HardDrive,
    CPU => Cpu,
    NETWORK => Network,
    SERVER => Server,
    PACKAGE => Package,
    LAYERS => Layers,
    DATABASE => Database,
    USB => Usb,
    CABLE => Cable,
    CLOCK => Timer,
    BRANCH => GitBranch,
    FILE => FileText,
    LOG => SquareTerminal,
    TERMINAL => Terminal,
    KEY => KeyRound,
    LOCK => Lock,
    BUG => Bug,
    HEARTBEAT => HeartPulse,
    POWER => Power,
    ROCKET => Rocket,
    SPARKLES => Sparkles,
    OFFLINE => CloudOff,
    ONLINE => Cloud,
    // theme
    THEME_LIGHT => Sun,
    THEME_DARK => Moon,
    THEME_SYSTEM => Monitor,
}

/// A Lucide icon at the default size (16 px, current text colour).
pub fn icon(name: IconName) -> Icon {
    Icon::new(name).size(px(16.0))
}

/// A Lucide icon with an explicit size and colour (status icons take the tone's colour).
pub fn icon_sized(name: IconName, size: f32, color: Hsla) -> Icon {
    Icon::new(name).size(px(size)).text_color(color)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::AssetSource;

    #[test]
    fn every_role_resolves_to_a_bundled_svg() {
        for (role, name) in ALL {
            let path = name.path();
            let bytes = RoleIcons.load(&path).unwrap().unwrap_or_else(|| panic!("{role}: {path} is not bundled"));
            let svg = std::str::from_utf8(&bytes).unwrap();
            assert!(svg.contains("<svg") && svg.contains("currentColor"), "{role}: {path} must be a recolourable Lucide svg");
        }
    }

    #[test]
    fn only_a_small_part_of_the_catalogue_is_embedded() {
        let total: usize = ALL.iter().filter_map(|(_, n)| RoleIcons.load(&n.path()).ok().flatten()).map(|b| b.len()).sum();
        assert!(total < 120 * 1024, "icons embed {total} bytes; keep the vocabulary small");
    }
}
