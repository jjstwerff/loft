// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN129 arc B — the SOURCE seam: what a lazy collection consults on a miss.
//
// Arc A had one source (a `.store` image) and reached it inline from
// `fetch_missing`. Arc B adds a second (a relational database), and the two
// differ in every detail except the question they answer — so the question is
// what lives here, and the details live in one implementation each.

use crate::database::{Parts, Stores};
use crate::keys::{Content, DbRef};

/// What a lazy collection is bound to, derived from the binding's source string.
///
/// Derived rather than declared: `store_bind_lazy` takes one string, and which
/// kind of source it names is a fact about the string. That keeps the loft
/// surface at one function and lets a new source arrive without a new builtin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LazySource {
    /// A `.store` image — a local path or an `http(s)` URL read by page
    /// (`REMOTE_STORES.md`). loft's own bytes, so a fetch is a range read.
    File(String),
    /// A relational database, named by a driver prefix (`sqlite:people.db`). A
    /// foreign schema, so a fetch is a derived query and a row to materialise.
    #[cfg(feature = "native-extensions")]
    Sql(crate::database::sql_source::Driver, String),
}

impl LazySource {
    /// Classify a binding's source string.
    #[must_use]
    pub fn of(source: &str) -> LazySource {
        #[cfg(feature = "native-extensions")]
        if let Some((driver, target)) = crate::database::sql_source::driver_of(source) {
            return LazySource::Sql(driver, target.to_string());
        }
        LazySource::File(source.to_string())
    }
}

/// The answer a source gives to *"fetch the entry for this key"* — three
/// outcomes, because two of them are the reason arc C exists.
///
/// `Absent` and `Unreachable` are the same `null` to a loft program and must
/// never be the same fact here: "no such person" is stable and true, "the source
/// is down" is neither (README failure path 1).
#[derive(Debug)]
pub enum Fetched {
    /// The entry is now IN the collection. The caller re-runs the lookup rather
    /// than trusting a ref from here — the collection stays the only authority
    /// on what is resident.
    Inserted,
    /// The source was reached and does not hold this key.
    Absent,
    /// The source could not be consulted, or cannot serve this collection at
    /// all; the string says why, and it is what a program reads back through
    /// `store_lazy_error`.
    Unreachable(String),
}

impl Stores {
    /// Ask one source for one key. The dispatch that makes the seam a seam.
    pub(crate) fn fetch_from_source(
        &mut self,
        data: &DbRef,
        db: u16,
        src: &LazySource,
        key: &[Content],
    ) -> Fetched {
        match src {
            #[cfg(paged_store)]
            LazySource::File(path) => self.fetch_from_file(path, data, key),
            #[cfg(not(paged_store))]
            LazySource::File(path) => Fetched::Unreachable(format!(
                "`{path}` needs the paged reader, which this build does not have"
            )),
            #[cfg(feature = "native-extensions")]
            LazySource::Sql(driver, target) => self.fetch_from_sql(*driver, target, data, db, key),
        }
    }

    /// The `.store` image source — arc A's fetch, unchanged.
    ///
    /// The order is load-bearing and predates the seam: TRY the fetch, and only
    /// on failure ask whether the source was reachable at all. Asking first
    /// would put a stat on the path of every successful fault.
    #[cfg(paged_store)]
    fn fetch_from_file(&mut self, path: &str, data: &DbRef, key: &[Content]) -> Fetched {
        // One key, whichever spelling it arrived in. A composite key is not
        // fetchable yet — @PLN125's associated types are what would carry it —
        // so it is left to answer absent rather than fetching the wrong row.
        let hit = match key {
            [Content::Long(k)] => self.load_key(data, path, *k),
            [Content::Str(s)] => self.load_key_text(data, path, s.str()),
            _ => false,
        };
        if hit {
            return Fetched::Inserted;
        }
        match crate::paged_reader::PageSource::open(path) {
            Err(reason) => Fetched::Unreachable(reason),
            // Reached it and the key was not there — a genuine absence.
            Ok(_) => Fetched::Absent,
        }
    }

    /// The database source — steps 5 and 6: derive the query, run it, and
    /// materialise the row into the collection.
    ///
    /// The collection is asked FIRST (by `find_or_fetch`) and re-asked after,
    /// which is arc A's rule unchanged: this never hands a record back
    /// directly, so the collection stays the only authority on what is resident
    /// and identity keeps falling out of it.
    #[cfg(feature = "native-extensions")]
    fn fetch_from_sql(
        &mut self,
        driver: crate::database::sql_source::Driver,
        target: &str,
        data: &DbRef,
        db: u16,
        key: &[Content],
    ) -> Fetched {
        use crate::database::sql_query::{Mapping, QueryShape, derive_select};
        use crate::database::sql_source::SqlConn;

        // Only a hash materialises today. A `sorted`/`index` collection derives
        // its query fine (`sql_query.rs`) but is INSERTED into differently, and
        // an insert done the wrong way corrupts the structure rather than
        // failing — so the kind is refused here until step 8 wires its own.
        if !matches!(self.types[db as usize].parts, Parts::Hash(_, _)) {
            return Fetched::Unreachable(format!(
                "a `sqlite:` source serves `hash` collections; `{}` is not one",
                self.types[db as usize].name
            ));
        }
        let desc = self.layout_descriptor(&[db]);
        let Some(sql) = derive_select(&desc, db, &QueryShape::Equality, &Mapping::default()) else {
            return Fetched::Unreachable(format!(
                "no query can be derived for `{}` — its element has a field that is \
                 not a column",
                self.types[db as usize].name
            ));
        };
        if sql.params.len() != key.len() {
            return Fetched::Unreachable(format!(
                "`{}` is keyed on {} column(s) and the lookup gave {}",
                self.types[db as usize].name,
                sql.params.len(),
                key.len()
            ));
        }
        let binds: Vec<crate::database::sql_source::Cell> = key.iter().map(cell_of).collect();
        let conn = match SqlConn::open(driver, target) {
            Ok(c) => c,
            Err(why) => return Fetched::Unreachable(why),
        };
        // Step 7 — the schema is interrogated ONCE, and a bind it cannot serve
        // never answers a lookup.
        if let Some(why) = self.schema_refusal(data, db, &desc, &sql, &conn) {
            return Fetched::Unreachable(why);
        }
        let rows = match conn.query(&sql.text, &binds) {
            Ok(r) => r,
            // A query that does not RUN is unreachable, never an absence: a
            // renamed column and a missing person must not read the same.
            Err(why) => return Fetched::Unreachable(why),
        };
        let Some(row) = rows.into_iter().next() else {
            return Fetched::Absent;
        };
        if row.len() != sql.columns.len() {
            return Fetched::Unreachable(format!(
                "`{target}` answered {} column(s) where the query asked {}",
                row.len(),
                sql.columns.len()
            ));
        }
        self.materialise(data, db, &sql, &row);
        Fetched::Inserted
    }

    /// @PLN129 arc B2 — run an explicit predicate against this collection's
    /// bound source and materialise every row it matches INTO the collection.
    /// Returns how many records the collection gained.
    ///
    /// The escape hatch for what the keys cannot express — `name LIKE 'Ada%'`, a
    /// predicate on a non-key column. Those cannot be derived from a layout, and
    /// a scan-then-filter would silently read the table, so this is EXPLICIT and
    /// visible in the source (QUERIES.md § what it cannot express).
    ///
    /// It is not a side channel returning a detached result set: rows land in the
    /// collection, and a row whose key is already resident is SKIPPED rather than
    /// materialised again. That is the rule the whole model rests on — the
    /// collection is asked first, always — and it is what makes a person reached
    /// by `LIKE` and a person reached by navigation the same record.
    #[cfg(feature = "native-extensions")]
    pub fn lazy_query(&mut self, coll: &DbRef, condition: &str) -> i64 {
        use crate::database::sql_query::{Mapping, QueryShape, derive_select};
        use crate::database::sql_source::SqlConn;

        let slot = (coll.store_nr, coll.rec, coll.pos);
        let Some(source) = self.lazy_source(coll) else {
            return self.lazy_refuse(slot, "this collection is not bound to a source");
        };
        let LazySource::Sql(driver, target) = LazySource::of(&source) else {
            return self.lazy_refuse(slot, &format!("`{source}` is not a database source"));
        };
        // The collection's TYPE, from the store it was allocated into — a
        // `reference` argument carries no type, and every derived name comes
        // from this one.
        let db = self.allocations[coll.store_nr as usize].known_type;
        if db == u16::MAX || !matches!(self.types[db as usize].parts, Parts::Hash(_, _)) {
            return self.lazy_refuse(slot, "an explicit query serves a `hash` collection");
        }
        let desc = self.layout_descriptor(&[db]);
        let shape = QueryShape::Filter(condition.to_string());
        let Some(sql) = derive_select(&desc, db, &shape, &Mapping::default()) else {
            return self.lazy_refuse(
                slot,
                &format!(
                    "no query can be derived for `{}`",
                    self.types[db as usize].name
                ),
            );
        };
        let conn = match SqlConn::open(driver, &target) {
            Ok(c) => c,
            Err(why) => return self.lazy_refuse(slot, &why),
        };
        // An explicit predicate is NOT index-checked: the caller asked for
        // exactly this, a `LIKE` has no index to use, and refusing it would
        // remove the escape hatch this exists to be. The columns still have to
        // be there, and a query that does not run reports through arc C.
        let rows = match conn.query(&sql.text, &[]) {
            Ok(r) => r,
            Err(why) => return self.lazy_refuse(slot, &why),
        };
        let mut added = 0i64;
        for row in rows {
            if row.len() != sql.columns.len() {
                return self.lazy_refuse(slot, "the source answered a different column count");
            }
            let Some(key) = self.row_key(db, &desc, &sql, &row) else {
                return self.lazy_refuse(slot, "a row arrived without the collection's key");
            };
            // Ask the collection FIRST. Without this a person already fetched by
            // key would be materialised a second time, and `is_same` would answer
            // false for one obvious person (BINDING.md § the real cost).
            if self.find(coll, db, &key).rec != 0 {
                continue;
            }
            self.materialise(coll, db, &sql, &row);
            added += 1;
        }
        added
    }

    /// Record why an explicit query could not run, and answer "nothing arrived".
    ///
    /// Through arc C's channel rather than a return code: a count cannot carry a
    /// reason, and `0` is also what a predicate matching nothing answers.
    #[cfg(feature = "native-extensions")]
    fn lazy_refuse(&mut self, slot: (u16, u32, u32), why: &str) -> i64 {
        let entry = self.lazy_errors.entry(slot).or_insert((0, why.to_string()));
        entry.0 += 1;
        0
    }

    /// The collection key carried by one fetched row.
    ///
    /// Read through the FIELD's content type rather than the cell's, because
    /// `find` compares against `field_content`, which reads the stored field the
    /// same way — a key built from what SQLite happened to return would not
    /// compare equal to the identical record already resident.
    #[cfg(feature = "native-extensions")]
    fn row_key(
        &self,
        db: u16,
        desc: &crate::database::LayoutDesc,
        sql: &crate::database::sql_query::Sql,
        row: &crate::database::sql_source::Row,
    ) -> Option<Vec<Content>> {
        use crate::database::LayoutNode;
        use crate::database::sql_source::Cell;
        let Parts::Hash(elem, key_fields) = &self.types[db as usize].parts else {
            return None;
        };
        let LayoutNode::Record(fields) = desc.nodes.get(elem)? else {
            return None;
        };
        let mut key = Vec::with_capacity(key_fields.len());
        for k in key_fields {
            let at = sql.columns.iter().position(|c| c.field == *k as usize)?;
            let cell = row.get(at)?.as_ref();
            key.push(match fields.get(*k as usize)?.content {
                5 => Content::Str(crate::keys::Str::new(match cell {
                    Some(Cell::Text(s)) => s.as_str(),
                    _ => "",
                })),
                3 | 2 => Content::Float(match cell {
                    Some(Cell::Real(v)) => *v,
                    #[allow(clippy::cast_precision_loss)]
                    Some(Cell::Int(v)) => *v as f64,
                    _ => 0.0,
                }),
                _ => Content::Long(match cell {
                    Some(Cell::Int(v)) => *v,
                    #[allow(clippy::cast_possible_truncation)]
                    Some(Cell::Real(v)) => *v as i64,
                    Some(Cell::Text(s)) => s.parse().unwrap_or(0),
                    None => 0,
                }),
            });
        }
        Some(key)
    }

    /// @PLN129 arc B step 7 — why this schema cannot serve this collection, or
    /// `None`.
    ///
    /// Decided once per binding and remembered: nothing about a foreign schema
    /// changes between two lookups, so re-asking would spend a round trip per
    /// fault to learn the same thing.
    #[cfg(feature = "native-extensions")]
    fn schema_refusal(
        &mut self,
        data: &DbRef,
        db: u16,
        desc: &crate::database::LayoutDesc,
        sql: &crate::database::sql_query::Sql,
        conn: &crate::database::sql_source::SqlConn,
    ) -> Option<String> {
        use crate::database::SchemaCheck;
        let slot = (data.store_nr, data.rec, data.pos);
        match self.lazy_sources.get(&slot).map(|b| &b.check) {
            Some(SchemaCheck::Ok) => return None,
            Some(SchemaCheck::Refused(why)) => return Some(why.clone()),
            _ => {}
        }
        let verdict = self.interrogate_schema(db, desc, sql, conn);
        if let Some(b) = self.lazy_sources.get_mut(&slot) {
            b.check = match &verdict {
                None => SchemaCheck::Ok,
                Some(why) => SchemaCheck::Refused(why.clone()),
            };
        }
        verdict
    }

    /// Ask the source whether it can serve this query: the columns must exist
    /// with a compatible affinity, and an INDEX must serve the lookup.
    #[cfg(feature = "native-extensions")]
    fn interrogate_schema(
        &self,
        db: u16,
        desc: &crate::database::LayoutDesc,
        sql: &crate::database::sql_query::Sql,
        conn: &crate::database::sql_source::SqlConn,
    ) -> Option<String> {
        use crate::database::LayoutNode;
        use crate::database::sql_source::affinity;
        let content_tp = self.content(db);
        let Some(LayoutNode::Record(fields)) = desc.nodes.get(&content_tp) else {
            return Some("the element type is not a record".to_string());
        };
        let table = sql.text.split(" FROM ").nth(1)?.split(' ').next()?;
        let declared = match conn.columns_of(table) {
            Ok(c) if !c.is_empty() => c,
            Ok(_) => return Some(format!("the source has no table {table}")),
            Err(why) => return Some(why),
        };
        for sel in &sql.columns {
            // The column name as the query spells it, minus the quoting the
            // mapping added — the catalogue answers unquoted names.
            let want = sel.column.trim_matches(['"', '`']);
            let Some((_, tp)) = declared.iter().find(|(n, _)| n == want) else {
                return Some(format!("{table} has no column `{want}`"));
            };
            // Affinity, not the declared string: SQLite treats a declared type
            // as advice and uses the affinity it implies, so a check on the
            // string would be checking spelling rather than behaviour.
            let a = affinity(tp);
            let wrong = match fields[sel.field].content {
                // A number in a TEXT/BLOB column comes back as text and is
                // converted on the way in, which is a silent reinterpretation
                // of someone's data rather than a fetch.
                0..=4 => a == "TEXT" || a == "BLOB",
                5 => a == "INTEGER" || a == "REAL",
                _ => false,
            };
            if wrong {
                return Some(format!(
                    "{table}.{want} is {tp} ({a} affinity), which cannot hold `{}`",
                    desc.names
                        .get(&fields[sel.field].content)
                        .cloned()
                        .unwrap_or_default()
                ));
            }
        }
        conn.plan_is_indexed(&sql.text).err()
    }

    /// Write one fetched row into a fresh element record and link it into the
    /// collection.
    ///
    /// Through `record_new` + `hash::add` — the same pair `coll += [x]` uses —
    /// so a SQL arrival and an ordinary loft insert end in the same place, which
    /// is what makes "one record however it arrived" true rather than intended.
    #[cfg(feature = "native-extensions")]
    fn materialise(
        &mut self,
        data: &DbRef,
        db: u16,
        sql: &crate::database::sql_query::Sql,
        row: &crate::database::sql_source::Row,
    ) {
        let content_tp = self.content(db);
        let Parts::Struct(fields) = self.types[content_tp as usize].parts.clone() else {
            return;
        };
        let entry = self.record_new(data, db, u16::MAX);
        for (col, sel) in sql.columns.iter().enumerate() {
            let f = &fields[sel.field];
            let at = entry.pos + u32::from(f.position);
            write_cell(
                &mut self.allocations[entry.store_nr as usize],
                entry.rec,
                at,
                f.content,
                row[col].as_ref(),
            );
        }
        crate::hash::add(
            data,
            &entry,
            &mut self.allocations,
            &self.types[db as usize].keys,
        );
    }
}

/// A loft key value as a bound parameter.
#[cfg(feature = "native-extensions")]
fn cell_of(c: &Content) -> crate::database::sql_source::Cell {
    use crate::database::sql_source::Cell;
    match c {
        Content::Long(v) => Cell::Int(*v),
        Content::Float(v) => Cell::Real(*v),
        Content::Single(v) => Cell::Real(f64::from(*v)),
        Content::Str(s) => Cell::Text(s.str().to_string()),
    }
}

/// Write one column value into a record field.
///
/// The mirror of `Stores::field_content`, which READS a field by the same
/// content-type ids — 0 integer, 1 long, 2 single, 3 float, 4 boolean, 5 text,
/// 6 character. Keeping the two spelled the same way is what makes a fetched
/// record read back as the value the database holds.
///
/// A SQL `NULL` writes loft's null for that type rather than a zero: C80 says a
/// read never raises, so absence has to be a VALUE, and `0` is a number someone
/// may have meant.
#[cfg(feature = "native-extensions")]
fn write_cell(
    store: &mut crate::store::Store,
    rec: u32,
    at: u32,
    content: u16,
    cell: Option<&crate::database::sql_source::Cell>,
) {
    use crate::database::sql_source::Cell;
    let as_int = |c: Option<&Cell>| match c {
        Some(Cell::Int(v)) => Some(*v),
        // A column typed REAL or TEXT reaching an integer field is a schema
        // mismatch; take what converts and leave the rest null rather than
        // writing a number nobody stored.
        Some(Cell::Real(v)) => Some(*v as i64),
        Some(Cell::Text(s)) => s.parse::<i64>().ok(),
        None => None,
    };
    let as_float = |c: Option<&Cell>| match c {
        Some(Cell::Real(v)) => Some(*v),
        #[allow(clippy::cast_precision_loss)]
        Some(Cell::Int(v)) => Some(*v as f64),
        Some(Cell::Text(s)) => s.parse::<f64>().ok(),
        None => None,
    };
    match content {
        // integer / long — `i64::MIN` is loft's null for both.
        0 => {
            store.set_int(rec, at, as_int(cell).unwrap_or(i64::MIN));
        }
        1 => {
            store.set_long(rec, at, as_int(cell).unwrap_or(i64::MIN));
        }
        2 => {
            #[allow(clippy::cast_possible_truncation)]
            store.set_single(rec, at, as_float(cell).unwrap_or(f64::NAN) as f32);
        }
        3 => {
            store.set_float(rec, at, as_float(cell).unwrap_or(f64::NAN));
        }
        // boolean and character are byte- and word-sized, and both read back
        // through the same accessors `field_content` uses.
        4 => {
            store.set_byte(rec, at, 0, i32::from(as_int(cell).unwrap_or(0) != 0));
        }
        6 => {
            let ch = match cell {
                Some(Cell::Text(s)) => s.chars().next().map_or(0, u32::from),
                other => u32::try_from(as_int(other).unwrap_or(0)).unwrap_or(0),
            };
            store.set_u32_raw(rec, at, ch);
        }
        // text — a NULL leaves the pointer at 0, which IS loft's null text, so
        // `''` and NULL stay two different answers all the way in.
        5 => {
            let ptr = match cell {
                Some(Cell::Text(s)) => store.set_str(s),
                Some(Cell::Int(v)) => store.set_str(&v.to_string()),
                Some(Cell::Real(v)) => store.set_str(&v.to_string()),
                None => 0,
            };
            store.set_u32_raw(rec, at, ptr);
        }
        // Nothing else reaches here: `derive_select` refuses any field that is
        // not one of the above, so a type with one never produced a query.
        _ => {}
    }
}
