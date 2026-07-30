use std::fmt;

pub const CURRENT_LIBRARY_SCHEMA: u32 = 57;
pub const CURRENT_DATA_SCHEMA: u32 = 13;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DarktableSchema {
    library: u32,
    data: u32,
}

impl DarktableSchema {
    #[must_use]
    pub const fn new(library: u32, data: u32) -> Self {
        Self { library, data }
    }

    #[must_use]
    pub const fn library(self) -> u32 {
        self.library
    }

    #[must_use]
    pub const fn data(self) -> u32 {
        self.data
    }

    /// Returns explicit organization query plans for a supported schema.
    ///
    /// The plans intentionally select named columns and include source rowids;
    /// callers must never rely on SQLite's physical row order.
    ///
    /// # Errors
    ///
    /// Returns an error for zero, unsupported future, or otherwise invalid schema versions.
    pub fn organization_plans(self) -> Result<OrganizationQueryPlans, SchemaError> {
        if self.library == 0 || self.data == 0 {
            return Err(SchemaError::InvalidVersion {
                library: self.library,
                data: self.data,
            });
        }
        if self.library > CURRENT_LIBRARY_SCHEMA || self.data > CURRENT_DATA_SCHEMA {
            return Err(SchemaError::UnsupportedFuture {
                library: self.library,
                data: self.data,
            });
        }
        Ok(OrganizationQueryPlans {
            tags: QueryPlan::new(
                "data.tags",
                "SELECT id, name, synonyms, flags FROM data.tags ORDER BY id",
            ),
            assignments: if self.library >= 30 {
                QueryPlan::new(
                    "main.tagged_images",
                    "SELECT rowid, imgid, tagid, position FROM main.tagged_images ORDER BY rowid, imgid, tagid",
                )
            } else {
                QueryPlan::new(
                    "main.tagged_images",
                    "SELECT rowid, imgid, tagid, NULL AS position FROM main.tagged_images ORDER BY rowid, imgid, tagid",
                )
            },
            labels: QueryPlan::new(
                "main.color_labels",
                "SELECT rowid, imgid, color FROM main.color_labels ORDER BY rowid, imgid, color",
            ),
            images: QueryPlan::new(
                "main.images",
                "SELECT id, group_id, flags FROM main.images ORDER BY id, rowid",
            ),
        })
    }

    /// Returns named-column plans for Darktable history, module-order, image
    /// endpoint, and history-hash records. A missing optional plan means that
    /// the table or column was not present in that historical schema.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaError`] for invalid or newer-than-supported schemas.
    pub fn history_plans(self) -> Result<HistoryQueryPlans, SchemaError> {
        self.validate()?;
        let history = QueryPlan::new(
            "main.history",
            if self.library >= 38 {
                "SELECT rowid, imgid, num, module, operation, op_params, enabled, blendop_params, blendop_version, multi_priority, multi_name, multi_name_hand_edited FROM main.history ORDER BY imgid, num, rowid"
            } else {
                "SELECT rowid, imgid, num, module, operation, op_params, enabled, blendop_params, blendop_version, multi_priority, multi_name, 0 AS multi_name_hand_edited FROM main.history ORDER BY imgid, num, rowid"
            },
        );
        let images = QueryPlan::new(
            "main.images",
            if self.library >= 9 {
                "SELECT rowid, id, history_end FROM main.images ORDER BY id, rowid"
            } else {
                "SELECT rowid, id, NULL AS history_end FROM main.images ORDER BY id, rowid"
            },
        );
        let module_orders = (self.library >= 22).then(|| {
            QueryPlan::new(
                "main.module_order",
                "SELECT rowid, imgid, version, iop_list FROM main.module_order ORDER BY imgid, rowid",
            )
        });
        let hashes = (self.library >= 23).then(|| {
            QueryPlan::new(
                "main.history_hash",
                if self.library >= 25 {
                    "SELECT rowid, imgid, basic_hash, auto_hash, current_hash, mipmap_hash FROM main.history_hash ORDER BY imgid, rowid"
                } else {
                    "SELECT rowid, imgid, basic_hash, auto_hash, current_hash, NULL AS mipmap_hash FROM main.history_hash ORDER BY imgid, rowid"
                },
            )
        });
        Ok(HistoryQueryPlans {
            history,
            images,
            module_orders,
            hashes,
        })
    }

    fn validate(self) -> Result<(), SchemaError> {
        if self.library == 0 || self.data == 0 {
            return Err(SchemaError::InvalidVersion {
                library: self.library,
                data: self.data,
            });
        }
        if self.library > CURRENT_LIBRARY_SCHEMA || self.data > CURRENT_DATA_SCHEMA {
            return Err(SchemaError::UnsupportedFuture {
                library: self.library,
                data: self.data,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryPlan {
    table: &'static str,
    sql: &'static str,
}

impl QueryPlan {
    const fn new(table: &'static str, sql: &'static str) -> Self {
        Self { table, sql }
    }

    #[must_use]
    pub const fn table(&self) -> &'static str {
        self.table
    }

    #[must_use]
    pub const fn sql(&self) -> &'static str {
        self.sql
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrganizationQueryPlans {
    pub tags: QueryPlan,
    pub assignments: QueryPlan,
    pub labels: QueryPlan,
    pub images: QueryPlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryQueryPlans {
    pub history: QueryPlan,
    pub images: QueryPlan,
    pub module_orders: Option<QueryPlan>,
    pub hashes: Option<QueryPlan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaError {
    InvalidVersion { library: u32, data: u32 },
    UnsupportedFuture { library: u32, data: u32 },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVersion { library, data } => {
                write!(
                    formatter,
                    "invalid Darktable schema version library={library} data={data}"
                )
            }
            Self::UnsupportedFuture { library, data } => write!(
                formatter,
                "unsupported future Darktable schema version library={library} data={data}"
            ),
        }
    }
}

impl std::error::Error for SchemaError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_are_explicit_and_stable() {
        let plans = DarktableSchema::new(CURRENT_LIBRARY_SCHEMA, CURRENT_DATA_SCHEMA)
            .organization_plans()
            .unwrap();
        assert!(
            plans
                .tags
                .sql()
                .starts_with("SELECT id, name, synonyms, flags")
        );
        assert!(
            plans
                .assignments
                .sql()
                .contains("ORDER BY rowid, imgid, tagid")
        );
        assert!(!plans.images.sql().contains("SELECT *"));
    }

    #[test]
    fn old_tag_assignment_schema_is_still_explicit() {
        let plans = DarktableSchema::new(29, 1).organization_plans().unwrap();
        assert!(plans.assignments.sql().contains("NULL AS position"));
    }

    #[test]
    fn history_plans_cover_schema_additions_without_select_star() {
        let old = DarktableSchema::new(8, CURRENT_DATA_SCHEMA)
            .history_plans()
            .unwrap();
        assert!(old.history.sql().contains("0 AS multi_name_hand_edited"));
        assert!(old.images.sql().contains("NULL AS history_end"));
        assert!(old.module_orders.is_none());
        assert!(old.hashes.is_none());

        let current = DarktableSchema::new(CURRENT_LIBRARY_SCHEMA, CURRENT_DATA_SCHEMA)
            .history_plans()
            .unwrap();
        assert!(current.history.sql().contains("multi_name_hand_edited"));
        assert!(current.images.sql().contains("history_end"));
        assert!(current.module_orders.is_some());
        assert!(current.hashes.is_some());
        assert!(!current.history.sql().contains("SELECT *"));
    }
}
