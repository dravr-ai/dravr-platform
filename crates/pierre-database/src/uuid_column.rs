// ABOUTME: Bind and decode a row's own uuid column the way each backend stores it: hyphenated TEXT on SQLite, native uuid on Postgres
// ABOUTME: The persistence twin of TenantId/UserId for ids that have no domain newtype, so one shared statement serves both drivers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use uuid::Uuid;

/// A uuid column value whose sqlx encoding follows the backend's schema.
///
/// `SQLite` schemas declare uuid columns as `TEXT` holding the hyphenated
/// form, Postgres schemas as `uuid`. sqlx's own `Uuid` impl writes a 16-byte
/// `BLOB` on `SQLite`, which never matches a `TEXT` row, so a shared
/// statement cannot bind a bare [`Uuid`]. [`TenantId`] and [`UserId`] solve
/// this for the two ids that have a domain newtype; this wrapper does the
/// same for a row's own primary key.
///
/// [`TenantId`]: pierre_core::models::TenantId
/// [`UserId`]: pierre_core::models::UserId
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UuidColumn(pub Uuid);

impl From<UuidColumn> for Uuid {
    fn from(column: UuidColumn) -> Self {
        column.0
    }
}

mod sqlite_impl {
    use sqlx::encode::IsNull;
    use sqlx::error::BoxDynError;
    use sqlx::sqlite::{Sqlite, SqliteArgumentValue, SqliteTypeInfo, SqliteValueRef};
    use sqlx::{Decode, Encode, Type};
    use uuid::Uuid;

    use super::UuidColumn;

    impl Type<Sqlite> for UuidColumn {
        fn type_info() -> SqliteTypeInfo {
            <String as Type<Sqlite>>::type_info()
        }
    }

    impl<'q> Encode<'q, Sqlite> for UuidColumn {
        fn encode_by_ref(
            &self,
            buf: &mut Vec<SqliteArgumentValue<'q>>,
        ) -> Result<IsNull, BoxDynError> {
            <String as Encode<'q, Sqlite>>::encode(self.0.to_string(), buf)
        }
    }

    impl<'r> Decode<'r, Sqlite> for UuidColumn {
        fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
            let text = <String as Decode<'r, Sqlite>>::decode(value)?;
            Ok(Self(Uuid::parse_str(&text)?))
        }
    }
}

#[cfg(feature = "postgresql")]
mod postgres_impl {
    use sqlx::encode::IsNull;
    use sqlx::error::BoxDynError;
    use sqlx::postgres::{PgArgumentBuffer, PgTypeInfo, PgValueRef, Postgres};
    use sqlx::{Decode, Encode, Type};
    use uuid::Uuid;

    use super::UuidColumn;

    impl Type<Postgres> for UuidColumn {
        fn type_info() -> PgTypeInfo {
            <Uuid as Type<Postgres>>::type_info()
        }
    }

    impl<'q> Encode<'q, Postgres> for UuidColumn {
        fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
            <Uuid as Encode<'q, Postgres>>::encode_by_ref(&self.0, buf)
        }
    }

    impl<'r> Decode<'r, Postgres> for UuidColumn {
        fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
            Ok(Self(<Uuid as Decode<'r, Postgres>>::decode(value)?))
        }
    }
}
