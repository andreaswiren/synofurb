//! D1b Welcome guide: the pure step machine and the content table. No GPUI view here (see `screens/welcome.rs`), so
//! the rules are unit-tested directly.
//!
//! Five steps, one icon + one headline + at most two short lines each, details behind an info button:
//! 1 what synofurb does, 2 the six steps, 3 before you start, 4 the way back, 5 ready (Start).
//!
//! Closing rules (what "seen" means):
//! * Skip, or Esc / the close button on steps 1 to 4: the guide is dismissed and does not open again by itself;
//! * on the last step, whatever closes it (Start, Esc, the close button) follows the "Don't show again" box, which is
//!   ticked by default; unticked, the guide opens again at the next launch;
//! * it can always be replayed from Help or the menu, which never changes the stored flag back.

use super::copy::{self, Item};
use gpui_kit::assets::IconName;

/// Number of steps of the guide.
pub const STEP_COUNT: usize = 5;

/// What one step shows.
pub struct StepContent {
    pub icon: IconName,
    /// The icon is a warning (disks are erased): drawn in the warning text colour, still with a word next to it.
    pub warn: bool,
    pub title: &'static Item,
    /// At most two short lines.
    pub lines: &'static [&'static Item],
    /// The `(i)` button next to the lines (details behind a click), if any.
    pub info: Option<&'static Item>,
    /// Step 2 draws the six wizard steps as a row (same Stepper look as the window).
    pub flow: bool,
}

use super::icons::role;

pub static STEPS: [StepContent; STEP_COUNT] = [
    StepContent {
        icon: role::SERVER,
        warn: false,
        title: &copy::WELCOME_1_TITLE,
        lines: &[&copy::WELCOME_1_A, &copy::WELCOME_1_B],
        info: None,
        flow: false,
    },
    StepContent { icon: role::LAYERS, warn: false, title: &copy::WELCOME_2_TITLE, lines: &[&copy::WELCOME_2_A], info: None, flow: true },
    StepContent {
        icon: role::WARN,
        warn: true,
        title: &copy::WELCOME_3_TITLE,
        lines: &[&copy::WELCOME_3_A, &copy::WELCOME_3_B],
        info: Some(&copy::WELCOME_3_INFO),
        flow: false,
    },
    StepContent {
        icon: role::RECOVER,
        warn: false,
        title: &copy::WELCOME_4_TITLE,
        lines: &[&copy::WELCOME_4_A, &copy::WELCOME_4_B],
        info: Some(&copy::WELCOME_4_INFO),
        flow: false,
    },
    StepContent {
        icon: role::ROCKET,
        warn: false,
        title: &copy::WELCOME_5_TITLE,
        lines: &[&copy::WELCOME_5_A, &copy::WELCOME_5_B],
        info: None,
        flow: false,
    },
];

/// What a move did to the guide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moved {
    /// Still open (possibly on another step).
    Open,
    /// The guide closes now.
    Closed,
}

/// The guide's state: the step, the "Don't show again" box and how it ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guide {
    step: usize,
    dont_show: bool,
}

impl Default for Guide {
    fn default() -> Self {
        Guide::new()
    }
}

impl Guide {
    /// Step 1, "Don't show again" ticked.
    pub fn new() -> Guide {
        Guide { step: 0, dont_show: true }
    }

    /// Open on a given step (0-based, clamped) - the demo screens.
    pub fn at(step: usize) -> Guide {
        Guide { step: step.min(STEP_COUNT - 1), ..Guide::new() }
    }

    pub fn step(&self) -> usize {
        self.step
    }

    pub fn is_first(&self) -> bool {
        self.step == 0
    }

    pub fn is_last(&self) -> bool {
        self.step == STEP_COUNT - 1
    }

    pub fn content(&self) -> &'static StepContent {
        &STEPS[self.step]
    }

    pub fn dont_show(&self) -> bool {
        self.dont_show
    }

    pub fn set_dont_show(&mut self, v: bool) {
        self.dont_show = v;
    }

    /// Next (Start on the last step, which closes the guide).
    pub fn next(&mut self) -> Moved {
        if self.is_last() {
            Moved::Closed
        } else {
            self.step += 1;
            Moved::Open
        }
    }

    /// Back; nothing happens on the first step (the button is disabled there, with a reason).
    pub fn back(&mut self) {
        self.step = self.step.saturating_sub(1);
    }

    /// Why Back is unavailable, `None` when it works.
    pub fn back_reason(&self) -> Option<&'static str> {
        self.is_first().then_some(copy::WELCOME_FIRST.short)
    }

    /// Skip closes the guide from any step but the last (where Start is the way out).
    pub fn skip(&mut self) -> Moved {
        Moved::Closed
    }

    pub fn shows_skip(&self) -> bool {
        !self.is_last()
    }

    /// The "Don't show again" box belongs to the last step only.
    pub fn shows_checkbox(&self) -> bool {
        self.is_last()
    }

    /// The primary button of this step: Next, or Start on the last one.
    pub fn primary(&self) -> &'static Item {
        if self.is_last() { &copy::WELCOME_START } else { &copy::WELCOME_NEXT }
    }

    /// Arrow keys: Right goes on (never past the last step; Start is deliberate), Left goes back.
    pub fn key(&mut self, key: &str) -> bool {
        match key {
            "right" if !self.is_last() => {
                self.step += 1;
                true
            }
            "left" if !self.is_first() => {
                self.step -= 1;
                true
            }
            _ => false,
        }
    }

    /// Should closing now store "seen"? Yes, except on the last step with the box unticked. (Enter on Start also
    /// reaches the dialog's own close path first, so the rule cannot depend on which button ended it.)
    pub fn remember_on_close(&self) -> bool {
        !self.is_last() || self.dont_show
    }

    /// Words a person reads in this state: headline, lines, the visible buttons and the checkbox (dots and icons carry
    /// no words; info popovers are behind a click). The budget is 60, shell included (see the copy tests).
    pub fn visible_words(&self) -> usize {
        let c = self.content();
        let mut n = copy::words(c.title.short) + c.lines.iter().map(|i| copy::words(i.short)).sum::<usize>();
        n += copy::words(copy::BTN_BACK.short) + copy::words(self.primary().short);
        if self.shows_skip() {
            n += copy::words(copy::WELCOME_SKIP.short);
        }
        if self.shows_checkbox() {
            n += copy::words(copy::WELCOME_HIDE.short);
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_forward_and_back_and_stops_at_the_ends() {
        let mut g = Guide::new();
        assert!(g.is_first() && !g.is_last());
        assert_eq!(g.back_reason(), Some("Already at the start."));
        g.back();
        assert_eq!(g.step(), 0, "Back on the first step does nothing");
        for want in 1..STEP_COUNT {
            assert_eq!(g.next(), Moved::Open);
            assert_eq!(g.step(), want);
        }
        assert!(g.is_last());
        assert_eq!(g.back_reason(), None);
        g.back();
        assert_eq!(g.step(), STEP_COUNT - 2);
        assert_eq!(g.next(), Moved::Open);
        assert_eq!(g.next(), Moved::Closed, "Start on the last step closes the guide");
    }

    #[test]
    fn one_primary_per_step_start_only_at_the_end() {
        let mut g = Guide::new();
        for _ in 0..STEP_COUNT - 1 {
            assert_eq!(g.primary().short, "Next");
            assert!(g.shows_skip() && !g.shows_checkbox());
            g.next();
        }
        assert_eq!(g.primary().short, "Start");
        assert!(!g.shows_skip(), "Start is the way out of the last step");
        assert!(g.shows_checkbox(), "Don't show again belongs to the last step");
    }

    #[test]
    fn arrow_keys_move_but_never_start() {
        let mut g = Guide::new();
        assert!(!g.key("left"), "no step before the first");
        for _ in 0..STEP_COUNT - 1 {
            assert!(g.key("right"));
        }
        assert!(g.is_last());
        assert!(!g.key("right"), "Right does not press Start");
        assert!(g.key("left"));
        assert_eq!(g.step(), STEP_COUNT - 2);
        assert!(!g.key("enter") && !g.key("escape") && !g.key("tab"), "other keys are not ours");
        assert_eq!(Guide::at(99).step(), STEP_COUNT - 1, "demo step is clamped");
    }

    #[test]
    fn skip_and_escape_remember_the_last_step_follows_the_box() {
        let mut g = Guide::new();
        assert_eq!(g.skip(), Moved::Closed);
        assert!(g.remember_on_close(), "Skip / Esc on an early step: seen");
        let mut g = Guide::at(STEP_COUNT - 1);
        assert!(g.dont_show(), "ticked by default");
        assert_eq!(g.next(), Moved::Closed);
        assert!(g.remember_on_close(), "Start with the box ticked: seen");
        let mut g = Guide::at(STEP_COUNT - 1);
        g.set_dont_show(false);
        assert_eq!(g.next(), Moved::Closed);
        assert!(!g.remember_on_close(), "Start with the box cleared: shown again next launch");
        assert!(!g.remember_on_close(), "and so is Esc on that step: the visible box decides");
        g.set_dont_show(true);
        assert!(g.remember_on_close());
        let mut g = Guide::at(1);
        g.set_dont_show(false);
        assert!(g.remember_on_close(), "the box is not shown before the last step, so it cannot hold the guide open");
    }

    #[test]
    fn every_step_has_an_icon_a_headline_and_at_most_two_lines() {
        for (i, s) in STEPS.iter().enumerate() {
            assert!(!s.title.short.is_empty(), "step {}", i + 1);
            assert!((1..=2).contains(&s.lines.len()), "step {}: {} lines", i + 1, s.lines.len());
            assert!(s.title.short.len() < 40);
        }
        assert_eq!(STEPS.iter().filter(|s| s.flow).count(), 1, "only step 2 draws the six-step row");
        assert!(STEPS[1].flow);
        assert!(STEPS[2].warn && STEPS.iter().filter(|s| s.warn).count() == 1, "only 'erased' is a warning");
    }

    #[test]
    fn the_safety_content_of_the_old_first_launch_dialog_is_kept() {
        let all: String = STEPS
            .iter()
            .flat_map(|s| s.lines.iter().map(|l| l.short).chain(s.info.map(|i| i.tip)).chain(s.info.map(|i| i.body)))
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        assert!(all.contains("nothing changes until you confirm"), "read-only until you confirm");
        assert!(all.contains("only reads until you confirm"));
        assert!(all.contains("tested backup"));
        assert!(all.contains("erased"));
        assert!(all.contains("screen readers are not supported"));
        assert!(all.contains("command line"));
        assert!(all.contains("boot manager") && all.contains("f1"));
    }

    #[test]
    fn the_declared_screen_copy_matches_what_the_guide_shows() {
        for step in 0..STEP_COUNT {
            let name = if step == 0 { "welcome".to_string() } else { format!("welcome-{}", step + 1) };
            let declared = copy::SCREENS.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("{name} has no copy declaration"));
            let n: usize = declared
                .visible
                .iter()
                .filter(|i| i.key != "WELCOME_3_INFO" && i.key != "WELCOME_4_INFO")
                .map(|i| copy::words(i.short))
                .sum();
            assert_eq!(n, Guide::at(step).visible_words(), "{name}: declared words differ from the drawn ones");
        }
    }

    #[test]
    fn every_state_stays_inside_the_word_budget() {
        let shell: usize = copy::SHELL.iter().map(|i| copy::words(i.short)).sum();
        for step in 0..STEP_COUNT {
            let g = Guide::at(step);
            assert!(g.visible_words() <= 25, "step {}: {} words of its own", step + 1, g.visible_words());
            assert!(g.visible_words() + shell <= 60, "step {}: {} words with the shell", step + 1, g.visible_words() + shell);
            for l in g.content().lines {
                assert!(copy::words(l.short) <= 12, "{}", l.key);
            }
            if let Some(i) = g.content().info {
                assert_eq!(copy::sentences(i.tip), 1, "{}: one-sentence tooltip", i.key);
            }
        }
    }
}
