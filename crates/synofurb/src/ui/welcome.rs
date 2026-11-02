//! D1b Welcome guide: the pure step machine and the content table. No GPUI view here (see `screens/welcome.rs`), so
//! the rules are unit-tested directly.
//!
//! Five steps, one icon + one headline + at most two short lines each, details behind an info button:
//! 1 what synofurb does, 2 the six steps, 3 before you start, 4 if something goes wrong, 5 ready.
//!
//! What "seen" means (`UiSettings.welcome_done`): only the last step's "Don't show again" box decides.
//! * Skip, Esc, the close button or a click outside on steps 1 to 4 close the guide for this launch only; it opens
//!   again at the next launch;
//! * on the last step, whatever closes it (Find NAS, Esc, the close button) stores the box: ticked = seen, unticked =
//!   not seen (this also clears a flag set earlier);
//! * the first launch starts with the box ticked; a replay from Help or the menu starts with the stored flag, so the
//!   box always shows what is stored.

use super::copy::{self, Item};
use super::icons::role;
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
    /// A muted one-line note under the lines (step 1: the screen-reader statement).
    pub note: Option<&'static Item>,
    /// The `(i)` button behind the last line (details behind a click), if any.
    pub info: Option<&'static Item>,
    /// Step 2 draws the six wizard steps as a row (the window's Stepper look), each with a short caption.
    pub flow: bool,
}

/// The six wizard steps with what each one is, for step 2.
pub static FLOW: [(&Item, &Item); 6] = [
    (&copy::STEP_FIND, &copy::WELCOME_FLOW_FIND),
    (&copy::STEP_CONNECT, &copy::WELCOME_FLOW_CONNECT),
    (&copy::STEP_GOAL, &copy::WELCOME_FLOW_GOAL),
    (&copy::STEP_SETUP, &copy::WELCOME_FLOW_SETUP),
    (&copy::STEP_CONFIRM, &copy::WELCOME_FLOW_CONFIRM),
    (&copy::STEP_INSTALL, &copy::WELCOME_FLOW_INSTALL),
];

pub static STEPS: [StepContent; STEP_COUNT] = [
    StepContent {
        icon: role::SERVER,
        warn: false,
        title: &copy::WELCOME_1_TITLE,
        lines: &[&copy::WELCOME_1_A, &copy::WELCOME_1_B],
        note: Some(&copy::WELCOME_1_NOTE),
        info: Some(&copy::WELCOME_1_INFO),
        flow: false,
    },
    StepContent {
        icon: role::LAYERS,
        warn: false,
        title: &copy::WELCOME_2_TITLE,
        lines: &[&copy::WELCOME_2_A, &copy::WELCOME_2_B],
        note: None,
        info: Some(&copy::WELCOME_2_INFO),
        flow: true,
    },
    StepContent {
        icon: role::WARN,
        warn: true,
        title: &copy::WELCOME_3_TITLE,
        lines: &[&copy::WELCOME_3_A, &copy::WELCOME_3_B],
        note: None,
        info: Some(&copy::WELCOME_3_INFO),
        flow: false,
    },
    StepContent {
        icon: role::RECOVER,
        warn: false,
        title: &copy::WELCOME_4_TITLE,
        lines: &[&copy::WELCOME_4_A, &copy::WELCOME_4_B],
        note: None,
        info: Some(&copy::WELCOME_4_INFO),
        flow: false,
    },
    StepContent {
        icon: role::ROCKET,
        warn: false,
        title: &copy::WELCOME_5_TITLE,
        lines: &[&copy::WELCOME_5_A, &copy::WELCOME_5_B],
        note: None,
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

/// The guide's state: the step and the "Don't show again" box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guide {
    step: usize,
    dont_show: bool,
}

impl Default for Guide {
    fn default() -> Self {
        Guide::new(0, true)
    }
}

impl Guide {
    /// Open on a step (0-based, clamped) with the box in the given state: ticked on the first launch, the stored flag
    /// on a replay.
    pub fn new(step: usize, dont_show: bool) -> Guide {
        Guide { step: step.min(STEP_COUNT - 1), dont_show }
    }

    /// First-launch state on a given step (the demo screens).
    pub fn at(step: usize) -> Guide {
        Guide::new(step, true)
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

    /// Jump to a step (the dots); out-of-range values are ignored.
    pub fn go_to(&mut self, step: usize) {
        if step < STEP_COUNT {
            self.step = step;
        }
    }

    /// Next (Find NAS on the last step, which closes the guide).
    pub fn next(&mut self) -> Moved {
        if self.is_last() {
            Moved::Closed
        } else {
            self.step += 1;
            Moved::Open
        }
    }

    /// Back; nothing happens on the first step, where the button is not drawn.
    pub fn back(&mut self) {
        self.step = self.step.saturating_sub(1);
    }

    pub fn shows_back(&self) -> bool {
        !self.is_first()
    }

    /// Skip closes the guide for this launch (steps 1 to 4); on the last step the primary is the way out.
    pub fn skip(&self) -> Moved {
        Moved::Closed
    }

    pub fn shows_skip(&self) -> bool {
        !self.is_last()
    }

    /// The "Don't show again" box belongs to the last step only.
    pub fn shows_checkbox(&self) -> bool {
        self.is_last()
    }

    /// The primary button of this step: Next, or "Find NAS" on the last one.
    pub fn primary(&self) -> &'static Item {
        if self.is_last() { &copy::WELCOME_START } else { &copy::WELCOME_NEXT }
    }

    /// Arrow keys: Right goes on (never past the last step; the primary is deliberate), Left goes back.
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

    /// What closing now writes to `welcome_done`: `Some(value)` on the last step (the box decides, ticked or not),
    /// `None` before it (the guide returns at the next launch and nothing is stored). Enter on the primary also reaches
    /// the dialog's own close path first, so the rule depends on the step and the box, never on which control ended it.
    pub fn stores_on_close(&self) -> Option<bool> {
        self.is_last().then_some(self.dont_show)
    }

    /// Words a person reads in this state: headline, lines, note, the step captions, the visible buttons and the
    /// checkbox (icons and dots carry no words; info popovers are behind a click). Budget: 60 with the shell.
    pub fn visible_words(&self) -> usize {
        let c = self.content();
        let w = |i: &Item| copy::words(i.short);
        let mut n = w(c.title) + c.lines.iter().map(|i| w(i)).sum::<usize>() + c.note.map_or(0, w);
        if c.flow {
            n += FLOW.iter().map(|(_, cap)| w(cap)).sum::<usize>();
        }
        n += w(self.primary());
        if self.shows_back() {
            n += w(&copy::BTN_BACK);
        }
        if self.shows_skip() {
            n += w(&copy::WELCOME_SKIP);
        }
        if self.shows_checkbox() {
            n += w(&copy::WELCOME_HIDE);
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_forward_and_back_and_stops_at_the_ends() {
        let mut g = Guide::default();
        assert!(g.is_first() && !g.is_last());
        assert!(!g.shows_back(), "no Back on the first step");
        g.back();
        assert_eq!(g.step(), 0, "Back on the first step does nothing");
        for want in 1..STEP_COUNT {
            assert_eq!(g.next(), Moved::Open);
            assert_eq!(g.step(), want);
            assert!(g.shows_back());
        }
        assert!(g.is_last());
        g.back();
        assert_eq!(g.step(), STEP_COUNT - 2);
        assert_eq!(g.next(), Moved::Open);
        assert_eq!(g.next(), Moved::Closed, "the primary on the last step closes the guide");
        g.go_to(2);
        assert_eq!(g.step(), 2);
        g.go_to(99);
        assert_eq!(g.step(), 2, "out of range is ignored");
    }

    #[test]
    fn one_primary_per_step_the_last_one_is_find_nas() {
        let mut g = Guide::default();
        for _ in 0..STEP_COUNT - 1 {
            assert_eq!(g.primary().short, "Next");
            assert!(g.shows_skip() && !g.shows_checkbox());
            g.next();
        }
        assert_eq!(g.primary().short, "Find NAS");
        assert!(!g.shows_skip(), "the primary is the way out of the last step");
        assert!(g.shows_checkbox(), "Don't show again belongs to the last step");
    }

    #[test]
    fn arrow_keys_move_but_never_press_the_primary() {
        let mut g = Guide::default();
        assert!(!g.key("left"), "no step before the first");
        for _ in 0..STEP_COUNT - 1 {
            assert!(g.key("right"));
        }
        assert!(g.is_last());
        assert!(!g.key("right"), "Right does not close the guide");
        assert!(g.key("left"));
        assert_eq!(g.step(), STEP_COUNT - 2);
        assert!(!g.key("enter") && !g.key("escape") && !g.key("tab"), "other keys are not ours");
        assert_eq!(Guide::at(99).step(), STEP_COUNT - 1, "demo step is clamped");
    }

    #[test]
    fn closing_early_stores_nothing_only_the_last_steps_box_decides() {
        for step in 0..STEP_COUNT - 1 {
            for box_state in [true, false] {
                let g = Guide::new(step, box_state);
                assert_eq!(g.stores_on_close(), None, "step {} (Skip, Esc, X, backdrop): this launch only", step + 1);
            }
        }
        assert_eq!(Guide::default().skip(), Moved::Closed);
        let last = Guide::at(STEP_COUNT - 1);
        assert_eq!(last.stores_on_close(), Some(true), "ticked by default on the first launch");
        let mut unticked = last;
        unticked.set_dont_show(false);
        assert_eq!(unticked.stores_on_close(), Some(false), "unticked: shown again next launch, and a stored flag is cleared");
    }

    #[test]
    fn a_replay_shows_the_stored_flag_and_unticking_clears_it() {
        // stored flag set: the replay opens with the box ticked
        let mut g = Guide::new(STEP_COUNT - 1, true);
        assert!(g.dont_show());
        g.set_dont_show(false);
        assert_eq!(g.stores_on_close(), Some(false), "unticking on a replay writes false");
        // stored flag clear: the replay opens unticked and closing keeps it clear
        let g = Guide::new(STEP_COUNT - 1, false);
        assert!(!g.dont_show());
        assert_eq!(g.stores_on_close(), Some(false));
    }

    #[test]
    fn every_step_has_an_icon_a_headline_and_at_most_two_lines() {
        for (i, s) in STEPS.iter().enumerate() {
            assert!(!s.title.short.is_empty(), "step {}", i + 1);
            assert!((1..=2).contains(&s.lines.len()), "step {}: {} lines", i + 1, s.lines.len());
        }
        assert_eq!(STEPS.iter().filter(|s| s.flow).count(), 1, "only step 2 draws the six-step row");
        assert!(STEPS[1].flow);
        assert!(STEPS[2].warn && STEPS.iter().filter(|s| s.warn).count() == 1, "only 'erased' is a warning");
        assert!(STEPS[0].note.is_some(), "the screen-reader statement is visible on step 1");
        assert_eq!(FLOW.len(), 6);
    }

    #[test]
    fn the_flow_row_uses_the_six_step_names_of_the_window() {
        let names: Vec<&str> = FLOW.iter().map(|(l, _)| l.short).collect();
        assert_eq!(names, ["Find", "Connect", "Goal", "Setup", "Confirm", "Install"]);
        for (_, cap) in FLOW {
            assert!(copy::words(cap.short) <= 3, "{}: a caption is at most three words", cap.key);
        }
    }

    /// Every sentence a first-time reader must be able to trust, checked against `docs/RECOVERY.md`, `docs/BOOT.md` and
    /// the "How recovery works" dialog (`copy::DLG_RECOVERY`).
    #[test]
    fn the_safety_statements_are_visible_and_conditional() {
        let visible = |n: usize| -> String {
            STEPS[n].lines.iter().map(|l| l.short).chain(STEPS[n].note.map(|i| i.short)).collect::<Vec<_>>().join(" ").to_lowercase()
        };
        assert!(visible(0).contains("nothing changes until you confirm"));
        assert!(
            visible(0).contains("screen readers are not supported") && visible(0).contains("command line"),
            "visible, not behind a hover"
        );
        assert!(visible(1).contains("changes begin after confirm") && visible(1).contains("restarts"));
        assert!(visible(2).contains("erased") && visible(2).contains("back up your files") && visible(2).contains("test"));
        // step 4 is conditional: before the disks change / after that, never "DSM stays the default"
        let s4 = visible(3);
        assert!(
            s4.contains("before the disks change") && s4.contains("after that, dsm must be reinstalled") && s4.contains("keep your backup")
        );
        assert!(!s4.contains("restores it") && !s4.contains("stays the default"), "no unconditional promise");
        let body = STEPS[3].info.unwrap().body.to_lowercase();
        assert!(body.contains("default boot entry is debian"), "after a finished install the default is Debian");
        assert!(body.contains("point of no return") && body.contains("reinstall it from synology"));
        assert!(body.contains("never starts") && body.contains("power cycle") && body.contains("manual recovery"));
        assert!(body.contains("an image of this unit exists") && body.contains("answers"));
        assert!(STEPS[3].info.unwrap().tip.contains("Way back"), "points to the status-bar link");
        for s in &STEPS {
            for i in s.lines.iter().chain(s.info.iter()).chain(s.note.iter()) {
                let t = format!("{} {} {}", i.short, i.tip, i.body).to_lowercase();
                assert!(!t.contains("rescue shell"), "{}: the app has no rescue shell", i.key);
                assert!(!t.contains("boot manager"), "{}: Boot manager is not a way back and is not part of the guide", i.key);
            }
        }
    }

    #[test]
    fn every_state_stays_inside_the_word_budget() {
        let shell: usize = copy::SHELL.iter().map(|i| copy::words(i.short)).sum();
        for step in 0..STEP_COUNT {
            let g = Guide::at(step);
            assert!(g.visible_words() <= 36, "step {}: {} words of its own", step + 1, g.visible_words());
            assert!(g.visible_words() + shell <= 60, "step {}: {} words with the shell", step + 1, g.visible_words() + shell);
            for l in g.content().lines.iter().chain(g.content().note.iter()) {
                assert!(copy::words(l.short) <= 12, "{}", l.key);
            }
            if let Some(i) = g.content().info {
                assert_eq!(copy::sentences(i.tip), 1, "{}: one-sentence tooltip", i.key);
            }
        }
    }

    #[test]
    fn the_declared_screen_copy_matches_what_the_guide_shows() {
        for step in 0..STEP_COUNT {
            let name = if step == 0 { "welcome".to_string() } else { format!("welcome-{}", step + 1) };
            let declared = copy::SCREENS.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("{name} has no copy declaration"));
            let n: usize = declared.visible.iter().map(|i| copy::words(i.short)).sum();
            assert_eq!(n, Guide::at(step).visible_words(), "{name}: declared words differ from the drawn ones");
        }
    }
}
