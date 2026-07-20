#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrganizationAccountingEntry {
    pub id: &'static str,
    pub upstream_path: &'static str,
    pub upstream_symbols: &'static str,
    pub rust_owner: &'static str,
    pub status: &'static str,
}

/// Deterministic source accounting for every organization responsibility in
/// the migration slice. The architecture TOML is the review-facing mirror.
pub const SOURCE_ACCOUNTING: &[OrganizationAccountingEntry] = &[
    OrganizationAccountingEntry {
        id: "tag-identity",
        upstream_path: "src/common/database.c;src/common/tags.h",
        upstream_symbols: "data.tags.id/name",
        rust_owner: "rusttable-compat::TagRecord",
        status: "replaced",
    },
    OrganizationAccountingEntry {
        id: "tag-hierarchy",
        upstream_path: "src/common/tags.c",
        upstream_symbols: "path splitting/tree construction",
        rust_owner: "rusttable-compat::TagRecord",
        status: "replaced",
    },
    OrganizationAccountingEntry {
        id: "tag-synonyms",
        upstream_path: "src/common/tags.c",
        upstream_symbols: "dt_tag_get_synonyms/dt_tag_set_synonyms",
        rust_owner: "rusttable-compat::SynonymRecord",
        status: "replaced",
    },
    OrganizationAccountingEntry {
        id: "tag-flags",
        upstream_path: "src/common/tags.h",
        upstream_symbols: "dt_tag_flags_t",
        rust_owner: "rusttable-compat::TagFlags",
        status: "replaced",
    },
    OrganizationAccountingEntry {
        id: "tag-assignment",
        upstream_path: "src/common/database.c",
        upstream_symbols: "main.tagged_images",
        rust_owner: "rusttable-compat::TagAssignmentRecord",
        status: "replaced",
    },
    OrganizationAccountingEntry {
        id: "image-rating-rejection",
        upstream_path: "src/common/image.h;src/common/image.c",
        upstream_symbols: "DT_VIEW_RATINGS_MASK/DT_IMAGE_REJECTED",
        rust_owner: "rusttable-compat::ImageOrganizationRecord",
        status: "replaced",
    },
    OrganizationAccountingEntry {
        id: "color-label-rows",
        upstream_path: "src/common/colorlabels.h;src/common/database.c",
        upstream_symbols: "DT_COLORLABELS_*;main.color_labels",
        rust_owner: "rusttable-compat::ColorLabelRecord",
        status: "replaced",
    },
    OrganizationAccountingEntry {
        id: "image-groups",
        upstream_path: "src/common/image.c;src/common/database.c",
        upstream_symbols: "images.group_id",
        rust_owner: "rusttable-compat::GroupRecord",
        status: "replaced",
    },
    OrganizationAccountingEntry {
        id: "representatives",
        upstream_path: "src/common/image.c",
        upstream_symbols: "group_id == image id",
        rust_owner: "rusttable-compat::GroupRecord",
        status: "replaced",
    },
    OrganizationAccountingEntry {
        id: "raw-provenance",
        upstream_path: "src/common/database.c",
        upstream_symbols: "schema/row identity and source ordering",
        rust_owner: "rusttable-compat::SourceRowKey",
        status: "replaced",
    },
    OrganizationAccountingEntry {
        id: "bounded-validation",
        upstream_path: "src/common/tags.c;src/common/image.c",
        upstream_symbols: "hierarchy/group traversal",
        rust_owner: "rusttable-compat::OrganizationDecoder",
        status: "replaced",
    },
    OrganizationAccountingEntry {
        id: "native-query-plans",
        upstream_path: "src/common/database.c",
        upstream_symbols: "schema-versioned organization columns",
        rust_owner: "rusttable-sqlite-native::OrganizationQueryPlans",
        status: "replaced",
    },
    OrganizationAccountingEntry {
        id: "typed-catalog-commands",
        upstream_path: "src/common/ratings.c;src/common/colorlabels.c",
        upstream_symbols: "batch rating/reject/label mutations",
        rust_owner: "rusttable-catalog::CatalogCommand",
        status: "replaced",
    },
    OrganizationAccountingEntry {
        id: "catalog-index-projections",
        upstream_path: "src/common/collection.c",
        upstream_symbols: "rating/color-label collection predicates",
        rust_owner: "rusttable-catalog::CatalogQuery",
        status: "replaced",
    },
];
