use std::fmt;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, PartialEq)]
        pub struct $name(pub String);
        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.into())
            }
        }
        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

string_id!(DeploymentId);
string_id!(PersonSubjectId);
string_id!(HouseholdId);
string_id!(OwnershipArrangementId);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialOwnerBinding {
    pub deployment_id: String,
    pub provider: String,
    pub provider_principal: String,
    pub credential_owner_subject: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateAccount {
    pub provider: String,
    pub provider_account_id: String,
    pub name: String,
    pub account_type: String,
    pub currency: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnershipKind {
    Unknown,
    Approved,
}
impl OwnershipKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Approved => "approved",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttributionTarget {
    Person(PersonSubjectId),
    Household,
    Unassigned,
}

impl fmt::Display for AttributionTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Person(id) => write!(f, "person:{}", id.as_str()),
            Self::Household => f.write_str("household"),
            Self::Unassigned => f.write_str("unassigned"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MembershipEpoch {
    pub deployment_id: DeploymentId,
    pub household_id: HouseholdId,
    pub person_subject_id: PersonSubjectId,
    pub role: String,
    pub start_epoch: i64,
    pub end_epoch: Option<i64>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessEpoch {
    pub deployment_id: DeploymentId,
    pub household_id: HouseholdId,
    pub account_id: String,
    pub person_subject_id: PersonSubjectId,
    pub start_generation: i64,
    pub end_generation: Option<i64>,
}
