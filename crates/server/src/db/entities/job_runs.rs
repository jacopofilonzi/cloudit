//! `job_runs`: one row per run of a background job (migration 000009). `outcome` is
//! `JobOutcome::as_str`; `counts` a JSON object of numbers.

use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "job_runs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub job: String,
    pub instance_id: String,
    pub started_at: DateTimeUtc,
    pub finished_at: Option<DateTimeUtc>,
    pub outcome: String,
    pub counts: String,
    pub error: Option<String>,
}

impl ActiveModelBehavior for ActiveModel {}
