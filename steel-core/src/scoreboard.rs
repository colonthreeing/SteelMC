//! Server scoreboard state.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use steel_utils::locks::SyncRwLock;

/// Score holder name stored by the vanilla scoreboard.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScoreHolder {
    name: String,
}

impl ScoreHolder {
    /// Creates a score holder from its scoreboard name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Returns the scoreboard name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Scoreboard team metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreboardTeam {
    name: String,
}

impl ScoreboardTeam {
    /// Creates a scoreboard team.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    /// Returns the team name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Scoreboard objective metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreboardObjective {
    name: String,
    read_only: bool,
}

impl ScoreboardObjective {
    /// Creates a writable objective.
    #[must_use]
    pub fn writable(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            read_only: false,
        }
    }

    /// Creates an objective with explicit mutability.
    #[must_use]
    pub fn new(name: impl Into<String>, read_only: bool) -> Self {
        Self {
            name: name.into(),
            read_only,
        }
    }

    /// Returns the objective name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns whether scores for this objective are read-only.
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }
}

/// Scoreboard operation error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScoreboardError {
    /// Objective names may not be empty.
    EmptyObjectiveName,
    /// Team names may not be empty.
    EmptyTeamName,
    /// An objective already exists.
    DuplicateObjective(String),
    /// A team already exists.
    DuplicateTeam(String),
    /// The requested objective does not exist.
    MissingObjective(String),
    /// The requested team does not exist.
    MissingTeam(String),
    /// The objective cannot be written by commands.
    ReadOnlyObjective(String),
}

impl fmt::Display for ScoreboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyObjectiveName => write!(f, "objective name cannot be empty"),
            Self::EmptyTeamName => write!(f, "team name cannot be empty"),
            Self::DuplicateObjective(name) => write!(f, "objective '{name}' already exists"),
            Self::DuplicateTeam(name) => write!(f, "team '{name}' already exists"),
            Self::MissingObjective(name) => write!(f, "objective '{name}' does not exist"),
            Self::MissingTeam(name) => write!(f, "team '{name}' does not exist"),
            Self::ReadOnlyObjective(name) => write!(f, "objective '{name}' is read-only"),
        }
    }
}

impl Error for ScoreboardError {}

#[derive(Default)]
struct ScoreboardState {
    objectives: BTreeMap<String, ScoreboardObjective>,
    scores: BTreeMap<String, BTreeMap<String, i32>>,
    teams: BTreeMap<String, ScoreboardTeam>,
    holder_teams: BTreeMap<String, String>,
}

/// Server-level scoreboard.
#[derive(Default)]
pub struct Scoreboard {
    state: SyncRwLock<ScoreboardState>,
}

impl Scoreboard {
    /// Creates an empty scoreboard.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a writable objective.
    ///
    /// # Errors
    ///
    /// Returns an error if the objective name is empty or already exists.
    pub fn add_objective(
        &self,
        name: impl Into<String>,
    ) -> Result<ScoreboardObjective, ScoreboardError> {
        self.add_objective_with_read_only(name, false)
    }

    /// Adds an objective with explicit mutability.
    ///
    /// # Errors
    ///
    /// Returns an error if the objective name is empty or already exists.
    pub fn add_objective_with_read_only(
        &self,
        name: impl Into<String>,
        read_only: bool,
    ) -> Result<ScoreboardObjective, ScoreboardError> {
        let objective = ScoreboardObjective::new(name, read_only);
        if objective.name().is_empty() {
            return Err(ScoreboardError::EmptyObjectiveName);
        }

        let mut state = self.state.write();
        if state.objectives.contains_key(objective.name()) {
            return Err(ScoreboardError::DuplicateObjective(
                objective.name().to_owned(),
            ));
        }

        state
            .objectives
            .insert(objective.name().to_owned(), objective.to_owned());
        Ok(objective)
    }

    /// Returns an objective by name.
    #[must_use]
    pub fn objective(&self, name: &str) -> Option<ScoreboardObjective> {
        self.state.read().objectives.get(name).cloned()
    }

    /// Returns objective names in stable order.
    #[must_use]
    pub fn objective_names(&self) -> Vec<String> {
        self.state.read().objectives.keys().cloned().collect()
    }

    /// Adds a scoreboard team.
    ///
    /// # Errors
    ///
    /// Returns an error if the team name is empty or already exists.
    pub fn add_team(&self, name: impl Into<String>) -> Result<ScoreboardTeam, ScoreboardError> {
        let team = ScoreboardTeam::new(name);
        if team.name().is_empty() {
            return Err(ScoreboardError::EmptyTeamName);
        }

        let mut state = self.state.write();
        if state.teams.contains_key(team.name()) {
            return Err(ScoreboardError::DuplicateTeam(team.name().to_owned()));
        }

        state.teams.insert(team.name().to_owned(), team.to_owned());
        Ok(team)
    }

    /// Returns a team by name.
    #[must_use]
    pub fn team(&self, name: &str) -> Option<ScoreboardTeam> {
        self.state.read().teams.get(name).cloned()
    }

    /// Returns team names in stable order.
    #[must_use]
    pub fn team_names(&self) -> Vec<String> {
        self.state.read().teams.keys().cloned().collect()
    }

    /// Returns the team name for a score holder.
    #[must_use]
    pub fn holder_team_name(&self, holder: &ScoreHolder) -> Option<String> {
        self.state.read().holder_teams.get(holder.name()).cloned()
    }

    /// Adds a holder to a team, replacing any previous holder team.
    ///
    /// # Errors
    ///
    /// Returns an error if the team no longer exists.
    pub fn add_holder_to_team(
        &self,
        holder: &ScoreHolder,
        team: &ScoreboardTeam,
    ) -> Result<(), ScoreboardError> {
        let mut state = self.state.write();
        if !state.teams.contains_key(team.name()) {
            return Err(ScoreboardError::MissingTeam(team.name().to_owned()));
        }

        state
            .holder_teams
            .insert(holder.name().to_owned(), team.name().to_owned());
        Ok(())
    }

    /// Returns tracked score holder names in stable order.
    #[must_use]
    pub fn tracked_holders(&self) -> Vec<ScoreHolder> {
        self.state
            .read()
            .scores
            .keys()
            .map(|name| ScoreHolder::new(name.to_owned()))
            .collect()
    }

    /// Returns the score for a holder/objective pair.
    #[must_use]
    pub fn score(&self, holder: &ScoreHolder, objective: &ScoreboardObjective) -> Option<i32> {
        self.state
            .read()
            .scores
            .get(holder.name())
            .and_then(|scores| scores.get(objective.name()).copied())
    }

    /// Sets a holder/objective score, creating the holder score entry if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the objective no longer exists or is read-only.
    pub fn set_score(
        &self,
        holder: &ScoreHolder,
        objective: &ScoreboardObjective,
        value: i32,
    ) -> Result<(), ScoreboardError> {
        let mut state = self.state.write();
        let Some(stored_objective) = state.objectives.get(objective.name()) else {
            return Err(ScoreboardError::MissingObjective(
                objective.name().to_owned(),
            ));
        };
        if stored_objective.is_read_only() {
            return Err(ScoreboardError::ReadOnlyObjective(
                objective.name().to_owned(),
            ));
        }

        state
            .scores
            .entry(holder.name().to_owned())
            .or_default()
            .insert(objective.name().to_owned(), value);
        Ok(())
    }

    /// Removes a holder/objective score if present.
    pub fn remove_score(&self, holder: &ScoreHolder, objective: &ScoreboardObjective) {
        let mut state = self.state.write();
        if let Some(scores) = state.scores.get_mut(holder.name()) {
            scores.remove(objective.name());
        }
    }

    /// Returns objective names that have a score for the holder.
    #[must_use]
    pub fn holder_objectives(&self, holder: &ScoreHolder) -> BTreeSet<String> {
        self.state
            .read()
            .scores
            .get(holder.name())
            .map_or_else(BTreeSet::new, |scores| scores.keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{ScoreHolder, Scoreboard, ScoreboardError, ScoreboardTeam};

    #[test]
    fn score_is_missing_until_set() {
        let scoreboard = Scoreboard::new();
        let objective = scoreboard
            .add_objective("kills")
            .expect("objective should be added");
        let holder = ScoreHolder::new("Steve");

        assert_eq!(scoreboard.score(&holder, &objective), None);

        scoreboard
            .set_score(&holder, &objective, 7)
            .expect("score should be writable");
        assert_eq!(scoreboard.score(&holder, &objective), Some(7));
    }

    #[test]
    fn duplicate_objective_is_rejected() {
        let scoreboard = Scoreboard::new();
        scoreboard
            .add_objective("kills")
            .expect("first objective should be added");

        assert_eq!(
            scoreboard.add_objective("kills"),
            Err(ScoreboardError::DuplicateObjective("kills".to_owned()))
        );
    }

    #[test]
    fn read_only_objective_rejects_score_writes() {
        let scoreboard = Scoreboard::new();
        let objective = scoreboard
            .add_objective_with_read_only("health", true)
            .expect("objective should be added");
        let holder = ScoreHolder::new("Steve");

        assert_eq!(
            scoreboard.set_score(&holder, &objective, 20),
            Err(ScoreboardError::ReadOnlyObjective("health".to_owned()))
        );
    }

    #[test]
    fn duplicate_team_is_rejected() {
        let scoreboard = Scoreboard::new();
        scoreboard
            .add_team("red")
            .expect("first team should be added");

        assert_eq!(
            scoreboard.add_team("red"),
            Err(ScoreboardError::DuplicateTeam("red".to_owned()))
        );
    }

    #[test]
    fn holder_team_assignment_replaces_previous_team() {
        let scoreboard = Scoreboard::new();
        let red = scoreboard
            .add_team("red")
            .expect("red team should be added");
        let blue = scoreboard
            .add_team("blue")
            .expect("blue team should be added");
        let holder = ScoreHolder::new("Steve");

        scoreboard
            .add_holder_to_team(&holder, &red)
            .expect("holder should join red team");
        assert_eq!(scoreboard.holder_team_name(&holder).as_deref(), Some("red"));

        scoreboard
            .add_holder_to_team(&holder, &blue)
            .expect("holder should move to blue team");
        assert_eq!(
            scoreboard.holder_team_name(&holder).as_deref(),
            Some("blue")
        );
    }

    #[test]
    fn missing_team_rejects_holder_assignment() {
        let scoreboard = Scoreboard::new();
        let holder = ScoreHolder::new("Steve");
        let missing = ScoreboardTeam::new("red");

        assert_eq!(
            scoreboard.add_holder_to_team(&holder, &missing),
            Err(ScoreboardError::MissingTeam("red".to_owned()))
        );
    }
}
