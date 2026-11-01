use crate::DomainError;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinanceNote {
    description: String,
    day: i32,
}
impl FinanceNote {
    pub fn new(description: &str, day: i32) -> Result<Self, DomainError> {
        let description = description.trim();
        if description.is_empty()
            || description.chars().count() > 200
            || description.chars().any(char::is_control)
            || !(1..=3_652_059).contains(&day)
        {
            return Err(DomainError::InvalidAccount);
        }
        Ok(Self {
            description: description.into(),
            day,
        })
    }
    pub fn description(&self) -> &str {
        &self.description
    }
    pub fn day(&self) -> i32 {
        self.day
    }
}
