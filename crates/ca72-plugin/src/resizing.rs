//! GUI-only resize transactions. A queued VST3 answer can arrive after another click or
//! another opening, so replies identify the request rather than just its repeated size.

use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct View {
    pub width: u32,
    pub saved_width: u32,
    pub expanded: bool,
    pub panel: bool,
    pub applied: bool,
    pub presets: bool,
}

#[derive(Debug)]
struct Request {
    size: (u32, u32),
    view: Option<View>,
    answer: Option<bool>,
}

#[derive(Debug, Default)]
struct State {
    serial: u64,
    confirmed: Option<View>,
    requests: BTreeMap<u64, Request>,
    restore: Option<View>,
    remembered_width: Option<u32>,
}

#[derive(Debug, Default)]
pub(crate) struct Resizing(Mutex<State>);

impl Resizing {
    pub fn opening(&self, view: View) {
        let mut s = self.0.lock().expect("resize state");
        s.serial += 1;
        s.confirmed = Some(view);
        s.requests.clear();
        s.restore = None;
        s.remembered_width = None;
    }

    pub fn close(&self) {
        let mut s = self.0.lock().expect("resize state");
        s.requests.clear();
        s.restore = None;
    }

    pub fn issue(&self, size: (u32, u32)) -> u64 {
        let mut s = self.0.lock().expect("resize state");
        s.serial += 1;
        let id = s.serial;
        s.requests.insert(
            id,
            Request {
                size,
                view: None,
                answer: None,
            },
        );
        id
    }

    pub fn requested(&self) -> Option<(u64, (u32, u32))> {
        let s = self.0.lock().expect("resize state");
        s.requests.get(&s.serial).map(|r| (s.serial, r.size))
    }

    pub fn finish(&self, view: View) {
        let mut s = self.0.lock().expect("resize state");
        let id = s.serial;
        if let Some(r) = s.requests.get_mut(&id) {
            // Freeze the completed action, before a later state recall can change it.
            if r.view.is_none() && view.panel == view.applied {
                r.view = Some(view);
            }
        } else if s.restore.is_none()
            && view.panel == view.applied
            && s.confirmed
                .is_some_and(|c| c.width == view.width && c.expanded == view.expanded)
        {
            // Presets can open without changing the full window's size. Keep that stable
            // non-size state so a subsequent refusal restores what the player was seeing.
            s.confirmed = Some(view);
        }
        Self::resolve(&mut s);
    }

    pub fn answer(&self, id: u64, accepted: bool) {
        let mut s = self.0.lock().expect("resize state");
        if let Some(r) = s.requests.get_mut(&id) {
            r.answer = Some(accepted);
            // Requests coalesced by the wrapper never reach the host. A newer completed
            // request supersedes them; a callback already executing completes first.
            s.requests.retain(|&k, _| k >= id);
            Self::resolve(&mut s);
        }
    }

    fn resolve(s: &mut State) {
        let done: Vec<_> = s
            .requests
            .iter()
            .filter_map(|(&id, r)| Some((id, r.view?, r.answer?)))
            .collect();
        for (id, view, accepted) in done {
            s.requests.remove(&id);
            if accepted {
                if s.confirmed.is_some_and(|old| old.width != view.width) && view.saved_width != 0 {
                    s.remembered_width = Some(view.saved_width);
                }
                s.confirmed = Some(view);
                if id == s.serial {
                    s.restore = None;
                }
            } else if id == s.serial {
                s.restore = s.confirmed;
            }
        }
    }

    /// Only a width change accepted by the host becomes the computer's default.
    pub fn take_remembered_width(&self) -> Option<u32> {
        self.0.lock().expect("resize state").remembered_width.take()
    }

    pub fn take_restore(&self) -> Option<View> {
        self.0.lock().expect("resize state").restore.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn view(expanded: bool) -> View {
        View {
            width: 861,
            saved_width: 0,
            expanded,
            panel: false,
            applied: false,
            presets: expanded,
        }
    }
    #[test]
    fn a_late_reply_cannot_undo_a_newer_request_or_opening() {
        let r = Resizing::default();
        r.opening(view(true));
        let a = r.issue((861, 288));
        r.finish(view(false));
        let b = r.issue((861, 492));
        r.finish(view(true));
        let c = r.issue((861, 288));
        r.finish(view(false));
        r.answer(a, false);
        assert_eq!(r.take_restore(), None);
        r.answer(b, true);
        assert_eq!(r.take_restore(), None);
        r.answer(c, false);
        assert_eq!(r.take_restore(), Some(view(true)));
        let old = r.issue((861, 288));
        r.finish(view(false));
        r.opening(view(true));
        r.answer(old, false);
        assert_eq!(r.take_restore(), None);
    }
    #[test]
    fn refusal_preserves_the_confirmed_preferences_and_automatic_width() {
        let r = Resizing::default();
        r.opening(view(true));
        let id = r.issue((861, 288));
        r.answer(id, false);
        r.finish(view(false));
        assert_eq!(r.take_restore(), Some(view(true)));
    }
}
