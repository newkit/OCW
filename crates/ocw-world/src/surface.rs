//! Riding surfaces and their rolling resistance (spec §5.1).

use crate::graph::RoadClass;

/// What the wheels roll on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Surface {
    /// Paved road (asphalt, concrete).
    #[default]
    Asphalt,
    /// Unpaved road: gravel, compacted dirt (tracks, paths).
    Gravel,
    /// Off the road: meadows, fields.
    Grass,
}

impl Surface {
    /// Factor on the rider's rolling resistance coefficient (asphalt = 1):
    /// typical Crr ~0.004 on asphalt, ~0.01 on gravel, ~0.025 on grass.
    pub fn rolling_factor(self) -> f64 {
        match self {
            Surface::Asphalt => 1.0,
            Surface::Gravel => 2.5,
            Surface::Grass => 6.0,
        }
    }

    /// The usual surface of a road class (without an OSM `surface` tag).
    pub fn of_class(class: RoadClass) -> Self {
        match class {
            RoadClass::Track | RoadClass::Path => Surface::Gravel,
            _ => Surface::Asphalt,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Surface::Asphalt => "asphalt",
            Surface::Gravel => "gravel",
            Surface::Grass => "grass",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Surface::Asphalt, Surface::Gravel, Surface::Grass]
            .into_iter()
            .find(|v| v.name() == s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rougher_surfaces_roll_harder() {
        assert_eq!(Surface::default(), Surface::Asphalt);
        assert!(Surface::Asphalt.rolling_factor() < Surface::Gravel.rolling_factor());
        assert!(Surface::Gravel.rolling_factor() < Surface::Grass.rolling_factor());
        assert_eq!(Surface::of_class(RoadClass::Track), Surface::Gravel);
        assert_eq!(Surface::of_class(RoadClass::Secondary), Surface::Asphalt);
        for s in [Surface::Asphalt, Surface::Gravel, Surface::Grass] {
            assert_eq!(Surface::parse(s.name()), Some(s));
        }
        assert_eq!(Surface::parse("ice"), None);
    }
}
