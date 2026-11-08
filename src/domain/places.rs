//! Places: search results, saved places (bookmarks) and recent destinations.

use super::geo::{distance_m, LatLon};

/// Two places closer than this are the same place.
pub const SAME_PLACE_M: f64 = 30.0;
pub const MAX_BOOKMARKS: usize = 30;

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    /// What it's called: the shop, the street and number, the town.
    pub name: String,
    /// Where it is, in a few words (street, town), shown under the name. Can be empty.
    pub detail: String,
    pub at: LatLon,
}

impl Place {
    pub fn new(name: &str, at: LatLon) -> Self {
        Self { name: name.to_string(), detail: String::new(), at }
    }

    /// The name and the detail together, as one line.
    pub fn full_name(&self) -> String {
        if self.detail.is_empty() { self.name.clone() } else { format!("{}, {}", self.name, self.detail) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookmarkKind {
    Home,
    Work,
    Other,
}

impl BookmarkKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Work => "work",
            Self::Other => "other",
        }
    }

    pub fn from_code(code: &str) -> Self {
        match code {
            "home" => Self::Home,
            "work" => Self::Work,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub kind: BookmarkKind,
    pub place: Place,
}

/// Saved places, Home and Work first, then the others in the order they were saved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bookmarks {
    pub items: Vec<Bookmark>,
}

impl Bookmarks {
    /// Saves a place. Home and Work are one each: saving a new one replaces the old. Saving a place that's already
    /// there under the same kind just renames it.
    pub fn save(&mut self, kind: BookmarkKind, place: Place) {
        if kind != BookmarkKind::Other {
            self.items.retain(|b| b.kind != kind);
        }
        self.items.retain(|b| !(b.kind == kind && distance_m(b.place.at, place.at) < SAME_PLACE_M));
        self.items.push(Bookmark { kind, place });
        self.items.sort_by_key(|b| match b.kind {
            BookmarkKind::Home => 0,
            BookmarkKind::Work => 1,
            BookmarkKind::Other => 2,
        });
        if self.items.len() > MAX_BOOKMARKS {
            self.items.truncate(MAX_BOOKMARKS);
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
        }
    }

    /// The saved place at (or very near) this spot.
    pub fn at(&self, at: LatLon) -> Option<usize> {
        self.items.iter().position(|b| distance_m(b.place.at, at) < SAME_PLACE_M)
    }

    pub fn of_kind(&self, kind: BookmarkKind) -> Option<&Bookmark> {
        self.items.iter().find(|b| b.kind == kind)
    }
}

/// Puts a place at the top of the recent destinations, without doubles, keeping `keep` of them.
pub fn remember_recent(recents: &mut Vec<Place>, place: &Place, keep: usize) {
    recents.retain(|p| p.name != place.name);
    recents.insert(0, place.clone());
    recents.truncate(keep);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_and_work_are_one_each_and_come_first() {
        let mut b = Bookmarks::default();
        b.save(BookmarkKind::Other, Place::new("Gym", (44.50, 11.30)));
        b.save(BookmarkKind::Home, Place::new("Home", (44.49, 11.34)));
        b.save(BookmarkKind::Home, Place::new("New home", (44.48, 11.35)));
        b.save(BookmarkKind::Work, Place::new("Pizzeria", (44.47, 11.33)));
        let names: Vec<_> = b.items.iter().map(|x| x.place.name.as_str()).collect();
        assert_eq!(names, ["New home", "Pizzeria", "Gym"]);
        assert_eq!(b.at((44.4800001, 11.35)), Some(0));
        assert_eq!(b.of_kind(BookmarkKind::Work).unwrap().place.name, "Pizzeria");
        b.save(BookmarkKind::Other, Place::new("Gym (renamed)", (44.50, 11.30)));
        assert_eq!(b.items.len(), 3);
        b.remove(2);
        assert_eq!(b.items.len(), 2);
    }

    #[test]
    fn recents_have_no_doubles() {
        let mut r = vec![Place::new("A", (1.0, 1.0)), Place::new("B", (2.0, 2.0))];
        remember_recent(&mut r, &Place::new("B", (2.0, 2.0)), 5);
        assert_eq!(r.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["B", "A"]);
    }
}
