//! A typed builder for ES|QL queries: each command a method, column names
//! quoted where they need it, and values sent as parameters rather than
//! written into the query.

use std::fmt;

use oxsearch_core::FieldValue;
use oxsearch_core::shapes::OneOrMany;

use crate::{ESQLParams, SingleOrMultiValue, async_query, query};

/// An ES|QL query, built a command at a time:
///
/// ```ignore
/// let query = Esql::from_index("articles")
///     .where_(col("year").gte(2020).and(col("title").matches("vector search")))
///     .keep(["title", "year"])
///     .sort([col("year").desc()])
///     .limit(10);
/// let rows: Vec<Article> = query.fetch(&client).await?;
/// ```
///
/// Values go into the request's `params`, each a `?` in the query, so a
/// value never changes the query's meaning. Expressions the builder has no
/// method for are written as text: `Expr::from("LENGTH(title) > 10")`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Esql {
    source: Option<String>,
    metadata: Vec<String>,
    commands: Vec<String>,
    params: Vec<SingleOrMultiValue>,
}

impl Esql {
    /// `FROM` an index, alias, data stream or pattern (`logs-*`).
    pub fn from_index(index: impl AsRef<str>) -> Esql {
        Esql::from_indices([index])
    }

    /// `FROM` several indices, aliases, data streams or patterns.
    pub fn from_indices<I: AsRef<str>>(indices: impl IntoIterator<Item = I>) -> Esql {
        let indices: Vec<String> = indices
            .into_iter()
            .map(|i| index_pattern(i.as_ref()))
            .collect();
        Esql {
            source: Some(format!("FROM {}", indices.join(", "))),
            ..Esql::default()
        }
    }

    /// `ROW`: one row of the given columns and values, without an index.
    pub fn row<N: AsRef<str>, V: Into<Param>>(columns: impl IntoIterator<Item = (N, V)>) -> Esql {
        let mut esql = Esql::default();
        let columns: Vec<String> = columns
            .into_iter()
            .map(|(name, value)| {
                esql.params.push(value.into().0);
                format!("{} = ?", ident(name.as_ref()))
            })
            .collect();
        esql.source = Some(format!("ROW {}", columns.join(", ")));
        esql
    }

    /// No source: the commands of one of [`Esql::fork`]'s branches.
    pub fn branch() -> Esql {
        Esql::default()
    }

    /// `METADATA` fields to read along with the documents' own, such as
    /// `_id`, `_index` and `_score`, which [`Esql::fuse`] needs. For a
    /// query `FROM` indices.
    pub fn metadata<I: AsRef<str>>(mut self, fields: impl IntoIterator<Item = I>) -> Esql {
        self.metadata
            .extend(fields.into_iter().map(|f| ident(f.as_ref())));
        self
    }

    /// `WHERE`: the rows for which the condition holds.
    pub fn where_(self, condition: impl Into<Expr>) -> Esql {
        let condition = condition.into();
        self.command("WHERE", condition)
    }

    /// `EVAL`: a column computed from the others, or replaced.
    pub fn eval(self, name: impl AsRef<str>, value: impl Into<Expr>) -> Esql {
        let value = value.into();
        let assignment = Expr {
            text: format!("{} = {}", ident(name.as_ref()), value.text),
            params: value.params,
        };
        self.command("EVAL", assignment)
    }

    /// `KEEP`: only these columns, in this order. A name may be a pattern
    /// (`author.*`).
    pub fn keep<I: AsRef<str>>(self, columns: impl IntoIterator<Item = I>) -> Esql {
        let columns = column_list(columns);
        self.command("KEEP", columns)
    }

    /// `DROP`: all columns but these. A name may be a pattern.
    pub fn drop<I: AsRef<str>>(self, columns: impl IntoIterator<Item = I>) -> Esql {
        let columns = column_list(columns);
        self.command("DROP", columns)
    }

    /// `RENAME`: a column under a new name.
    pub fn rename(self, from: impl AsRef<str>, to: impl AsRef<str>) -> Esql {
        let renaming = format!("{} AS {}", ident(from.as_ref()), ident(to.as_ref()));
        self.command("RENAME", renaming)
    }

    /// `SORT`: the rows in this order, by the first key, then the next.
    pub fn sort(self, keys: impl IntoIterator<Item = Order>) -> Esql {
        let keys: Vec<String> = keys.into_iter().map(|k| k.0).collect();
        self.command("SORT", keys.join(", "))
    }

    /// `LIMIT`: at most this many rows.
    pub fn limit(self, rows: u32) -> Esql {
        self.command("LIMIT", rows.to_string())
    }

    /// `STATS ... BY`: one row per group of the `by` columns, or one row for
    /// all when there are none, with the named aggregations:
    /// `.stats([("avg_price", "AVG(price)")], ["category"])`.
    pub fn stats<N, A, B>(
        self,
        aggregations: impl IntoIterator<Item = (N, A)>,
        by: impl IntoIterator<Item = B>,
    ) -> Esql
    where
        N: AsRef<str>,
        A: Into<Expr>,
        B: AsRef<str>,
    {
        let mut stats = Expr::default();
        let aggregations: Vec<String> = aggregations
            .into_iter()
            .map(|(name, aggregation)| {
                let aggregation = aggregation.into();
                stats.params.extend(aggregation.params);
                format!("{} = {}", ident(name.as_ref()), aggregation.text)
            })
            .collect();
        let by: Vec<String> = by.into_iter().map(|b| ident(b.as_ref())).collect();
        let mut parts = Vec::new();
        if !aggregations.is_empty() {
            parts.push(aggregations.join(", "));
        }
        if !by.is_empty() {
            parts.push(format!("BY {}", by.join(", ")));
        }
        stats.text = parts.join(" ");
        self.command("STATS", stats)
    }

    /// `MV_EXPAND`: a row for each value of a multi-valued column.
    pub fn mv_expand(self, column: impl AsRef<str>) -> Esql {
        let column = ident(column.as_ref());
        self.command("MV_EXPAND", column)
    }

    /// `FORK`: the rows so far through each branch, the branches' rows
    /// together after, with a `_fork` column naming the branch each came
    /// from (`fork1`, `fork2`, …). Build the branches with
    /// [`Esql::branch`].
    pub fn fork(mut self, branches: impl IntoIterator<Item = Esql>) -> Esql {
        let mut text = String::from("FORK");
        for branch in branches {
            text.push_str(" (");
            text.push_str(&branch.pipeline());
            text.push(')');
            self.params.extend(branch.params);
        }
        self.commands.push(text);
        self
    }

    /// `FUSE`: rows that [`Esql::fork`]'s branches found more than once,
    /// merged, and scored by reciprocal rank fusion. The query reads `_id`,
    /// `_index` and `_score` with [`Esql::metadata`].
    pub fn fuse(self) -> Esql {
        self.fuse_with(Fuse::default())
    }

    /// `FUSE` with a method and options.
    pub fn fuse_with(mut self, fuse: Fuse) -> Esql {
        let options = fuse.text();
        self.commands.push(if options.is_empty() {
            "FUSE".to_owned()
        } else {
            format!("FUSE {options}")
        });
        self
    }

    /// Any other command, as text: `.then("DISSECT message \"%{level} %{text}\"")`,
    /// with a parameter for each `?`.
    pub fn then(mut self, command: impl Into<Expr>) -> Esql {
        let command = command.into();
        self.commands.push(command.text);
        self.params.extend(command.params);
        self
    }

    /// The query's text.
    pub fn query(&self) -> String {
        let mut source = self.source.clone().unwrap_or_default();
        if !self.metadata.is_empty() {
            source = format!("{source} METADATA {}", self.metadata.join(", "));
        }
        let pipeline = self.pipeline();
        match (source.is_empty(), pipeline.is_empty()) {
            (true, _) => pipeline,
            (false, true) => source,
            (false, false) => format!("{source} | {pipeline}"),
        }
    }

    /// The values for the query's `?`s, in order.
    pub fn params(&self) -> &[SingleOrMultiValue] {
        &self.params
    }

    /// The request that runs the query.
    pub fn request(&self) -> query::Request {
        query::Request::new().body(self.body())
    }

    /// The body of a request that runs the query, for setting the
    /// request's other options: `query::Request::new().body(esql.body().time_zone("Europe/Paris"))`.
    pub fn body(&self) -> query::RequestBody {
        let mut body = query::RequestBody::new(self.query());
        if !self.params.is_empty() {
            body = body.params(ESQLParams::Value(self.params.clone()));
        }
        body
    }

    /// The body of a request that runs the query asynchronously:
    /// `async_query::Request::new().body(esql.async_body().keep_on_completion(true))`.
    pub fn async_body(&self) -> async_query::RequestBody {
        let mut body = async_query::RequestBody::new(self.query());
        if !self.params.is_empty() {
            body = body.params(ESQLParams::Value(self.params.clone()));
        }
        body
    }

    /// The commands after the source.
    fn pipeline(&self) -> String {
        self.commands.join(" | ")
    }

    fn command(mut self, name: &str, arguments: impl Into<Expr>) -> Esql {
        let arguments = arguments.into();
        self.commands.push(format!("{name} {}", arguments.text));
        self.params.extend(arguments.params);
        self
    }
}

impl fmt::Display for Esql {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.query())
    }
}

impl From<Esql> for query::Request {
    fn from(esql: Esql) -> query::Request {
        esql.request()
    }
}

/// A value sent as a query parameter: a number, a string, a boolean, or a
/// list of them for a multi-valued parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct Param(pub SingleOrMultiValue);

macro_rules! params {
    ($($t:ty => $convert:expr),* $(,)?) => {
        $(impl From<$t> for Param {
            fn from(value: $t) -> Param {
                let convert: fn($t) -> FieldValue = $convert;
                Param(OneOrMany::One(convert(value)))
            }
        })*
    };
}

params! {
    i32 => |v| FieldValue::Long(v.into()),
    i64 => FieldValue::Long,
    u32 => |v| FieldValue::Long(v.into()),
    f32 => |v| FieldValue::Double(v.into()),
    f64 => FieldValue::Double,
    bool => FieldValue::Boolean,
    &str => |v| FieldValue::String(v.to_owned()),
    String => FieldValue::String,
    FieldValue => |v| v,
}

impl<T: Into<Param>> From<Vec<T>> for Param {
    fn from(values: Vec<T>) -> Param {
        let values = values
            .into_iter()
            .flat_map(|v| match v.into().0 {
                OneOrMany::One(one) => vec![one],
                OneOrMany::Many(many) => many,
            })
            .collect();
        Param(OneOrMany::Many(values))
    }
}

/// An expression, as ES|QL text with a parameter for each `?`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Expr {
    text: String,
    params: Vec<SingleOrMultiValue>,
}

impl Expr {
    /// An expression with a value for each `?` in its text, in order:
    /// `Expr::new("price < ? * 2", [10])`.
    pub fn new<P: Into<Param>>(
        text: impl Into<String>,
        params: impl IntoIterator<Item = P>,
    ) -> Expr {
        Expr {
            text: text.into(),
            params: params.into_iter().map(|p| p.into().0).collect(),
        }
    }

    /// Both conditions.
    pub fn and(self, other: impl Into<Expr>) -> Expr {
        self.join("AND", other.into())
    }

    /// Either condition.
    pub fn or(self, other: impl Into<Expr>) -> Expr {
        self.join("OR", other.into())
    }

    /// The text, with `?` for each parameter.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The parameters, in order.
    pub fn params(&self) -> &[SingleOrMultiValue] {
        &self.params
    }

    fn join(mut self, operator: &str, other: Expr) -> Expr {
        self.text = format!("({}) {operator} ({})", self.text, other.text);
        self.params.extend(other.params);
        self
    }
}

/// The condition's opposite: `!col("deleted").eq(true)`.
impl std::ops::Not for Expr {
    type Output = Expr;

    fn not(self) -> Expr {
        Expr {
            text: format!("NOT ({})", self.text),
            params: self.params,
        }
    }
}

impl From<&str> for Expr {
    fn from(text: &str) -> Expr {
        Expr {
            text: text.to_owned(),
            params: Vec::new(),
        }
    }
}

impl From<String> for Expr {
    fn from(text: String) -> Expr {
        Expr {
            text,
            params: Vec::new(),
        }
    }
}

/// A column, for conditions and sort keys: `col("year").gte(2020)`.
pub fn col(name: impl AsRef<str>) -> Column {
    Column(ident(name.as_ref()))
}

/// A column, quoted if its name needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column(String);

impl Column {
    fn compare(&self, operator: &str, value: impl Into<Param>) -> Expr {
        Expr::new(format!("{} {operator} ?", self.0), [value])
    }

    /// Equal to the value.
    pub fn eq(&self, value: impl Into<Param>) -> Expr {
        self.compare("==", value)
    }

    /// Not equal to the value.
    pub fn ne(&self, value: impl Into<Param>) -> Expr {
        self.compare("!=", value)
    }

    /// Less than the value.
    pub fn lt(&self, value: impl Into<Param>) -> Expr {
        self.compare("<", value)
    }

    /// At most the value.
    pub fn lte(&self, value: impl Into<Param>) -> Expr {
        self.compare("<=", value)
    }

    /// More than the value.
    pub fn gt(&self, value: impl Into<Param>) -> Expr {
        self.compare(">", value)
    }

    /// At least the value.
    pub fn gte(&self, value: impl Into<Param>) -> Expr {
        self.compare(">=", value)
    }

    /// One of the values.
    pub fn in_<P: Into<Param>>(&self, values: impl IntoIterator<Item = P>) -> Expr {
        let params: Vec<Param> = values.into_iter().map(Into::into).collect();
        let marks = vec!["?"; params.len()].join(", ");
        Expr::new(format!("{} IN ({marks})", self.0), params)
    }

    /// Without a value.
    pub fn is_null(&self) -> Expr {
        Expr::from(format!("{} IS NULL", self.0))
    }

    /// With a value.
    pub fn is_not_null(&self) -> Expr {
        Expr::from(format!("{} IS NOT NULL", self.0))
    }

    /// Matching a wildcard pattern: `*` for any characters, `?` for one.
    pub fn like(&self, pattern: impl Into<String>) -> Expr {
        Expr::new(format!("{} LIKE ?", self.0), [pattern.into()])
    }

    /// Matching a regular expression.
    pub fn rlike(&self, pattern: impl Into<String>) -> Expr {
        Expr::new(format!("{} RLIKE ?", self.0), [pattern.into()])
    }

    /// A full-text match of the query against the field, as the `match`
    /// query does, which scores the rows (`_score`) and works on
    /// `semantic_text` fields too: `MATCH(column, query)`.
    pub fn matches(&self, query: impl Into<String>) -> Expr {
        Expr::new(format!("MATCH({}, ?)", self.0), [query.into()])
    }

    /// Ascending, nulls last.
    pub fn asc(&self) -> Order {
        Order(format!("{} ASC", self.0))
    }

    /// Descending, nulls first.
    pub fn desc(&self) -> Order {
        Order(format!("{} DESC", self.0))
    }
}

impl From<Column> for Expr {
    fn from(column: Column) -> Expr {
        Expr::from(column.0)
    }
}

/// A sort key: a column and a direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Order(String);

impl Order {
    /// Rows without a value first.
    pub fn nulls_first(self) -> Order {
        Order(format!("{} NULLS FIRST", self.0))
    }

    /// Rows without a value last.
    pub fn nulls_last(self) -> Order {
        Order(format!("{} NULLS LAST", self.0))
    }
}

/// How `FUSE` merges [`Esql::fork`]'s branches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fuse {
    method: Option<&'static str>,
    score_by: Option<String>,
    key_by: Vec<String>,
    group_by: Option<String>,
    options: Option<serde_json::Value>,
}

impl Fuse {
    /// Reciprocal rank fusion, the default: each row scores by its ranks in
    /// the branches.
    pub fn rrf() -> Fuse {
        Fuse {
            method: Some("RRF"),
            ..Fuse::default()
        }
    }

    /// A weighted sum of the branches' scores.
    pub fn linear() -> Fuse {
        Fuse {
            method: Some("LINEAR"),
            ..Fuse::default()
        }
    }

    /// The column with each branch's score, `_score` by default.
    pub fn score_by(mut self, column: impl AsRef<str>) -> Fuse {
        self.score_by = Some(ident(column.as_ref()));
        self
    }

    /// The columns that say two rows are the same, `_id` and `_index` by
    /// default.
    pub fn key_by<I: AsRef<str>>(mut self, columns: impl IntoIterator<Item = I>) -> Fuse {
        self.key_by = columns.into_iter().map(|c| ident(c.as_ref())).collect();
        self
    }

    /// The column naming each row's branch, `_fork` by default.
    pub fn group_by(mut self, column: impl AsRef<str>) -> Fuse {
        self.group_by = Some(ident(column.as_ref()));
        self
    }

    /// The method's options, such as `{"rank_constant": 20}` for RRF or
    /// `{"normalizer": "minmax", "weights": {"fork1": 0.7, "fork2": 0.3}}`
    /// for a linear combination.
    pub fn with(mut self, options: serde_json::Value) -> Fuse {
        self.options = Some(options);
        self
    }

    fn text(&self) -> String {
        let mut parts = Vec::new();
        if let Some(method) = self.method {
            parts.push(method.to_owned());
        }
        if let Some(score) = &self.score_by {
            parts.push(format!("SCORE BY {score}"));
        }
        if !self.key_by.is_empty() {
            parts.push(format!("KEY BY {}", self.key_by.join(", ")));
        }
        if let Some(group) = &self.group_by {
            parts.push(format!("GROUP BY {group}"));
        }
        if let Some(options) = &self.options {
            parts.push(format!("WITH {options}"));
        }
        parts.join(" ")
    }
}

/// ES|QL's keywords, which a column can't be named unquoted.
const KEYWORDS: &[&str] = &[
    "and", "as", "asc", "by", "desc", "false", "first", "in", "is", "last", "like", "metadata",
    "not", "null", "nulls", "on", "or", "rlike", "true", "with",
];

/// A column name as ES|QL reads it: as it is when it can be, else quoted
/// with backticks. A dotted name (`author.name`) is one column.
fn ident(name: &str) -> String {
    let plain = |part: &str| {
        let mut chars = part.chars();
        let first_ok = match chars.next() {
            Some(c) if c.is_ascii_alphabetic() => true,
            Some('_' | '@') => !part[1..].is_empty(),
            _ => false,
        };
        first_ok
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !KEYWORDS.contains(&part.to_ascii_lowercase().as_str())
    };
    if name.split('.').all(plain) {
        name.to_owned()
    } else {
        format!("`{}`", name.replace('`', "``"))
    }
}

/// Column names or patterns: a pattern's `*` stays unquoted.
fn column_list<I: AsRef<str>>(columns: impl IntoIterator<Item = I>) -> String {
    let columns: Vec<String> = columns
        .into_iter()
        .map(|c| {
            let c = c.as_ref();
            if c.contains('*') {
                c.to_owned()
            } else {
                ident(c)
            }
        })
        .collect();
    columns.join(", ")
}

/// An index name or pattern as `FROM` reads it: as it is when it can be,
/// else quoted.
fn index_pattern(index: &str) -> String {
    let plain = !index.is_empty()
        && index
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '*' | ':' | '+'));
    if plain {
        index.to_owned()
    } else {
        serde_json::to_string(index).expect("a string writes as JSON")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn params(esql: &Esql) -> serde_json::Value {
        serde_json::to_value(esql.params()).unwrap()
    }

    #[test]
    fn commands() {
        let query = Esql::from_indices(["articles", "logs-*"])
            .metadata(["_id", "_score"])
            .where_(
                col("year")
                    .gte(2020)
                    .and(col("title").matches("vector search")),
            )
            .eval("decade", "year - year % 10")
            .keep(["title", "year", "author.*"])
            .drop(["internal"])
            .rename("title", "headline")
            .sort([col("year").desc(), col("headline").asc().nulls_first()])
            .limit(10);
        assert_eq!(
            query.query(),
            "FROM articles, logs-* METADATA _id, _score \
             | WHERE (year >= ?) AND (MATCH(title, ?)) \
             | EVAL decade = year - year % 10 \
             | KEEP title, year, author.* \
             | DROP internal \
             | RENAME title AS headline \
             | SORT year DESC, headline ASC NULLS FIRST \
             | LIMIT 10"
        );
        assert_eq!(params(&query), json!([2020, "vector search"]));
    }

    #[test]
    fn names_and_values() {
        let query = Esql::row([
            ("a", Param::from(1)),
            ("my col", Param::from(vec!["x", "y"])),
        ])
        .where_(
            col("@timestamp")
                .is_not_null()
                .or(!col("by").in_(["a", "b"])),
        )
        .stats([("n", "COUNT(*)")], ["author.name", "first-name"]);
        assert_eq!(
            query.query(),
            "ROW a = ?, `my col` = ? \
             | WHERE (@timestamp IS NOT NULL) OR (NOT (`by` IN (?, ?))) \
             | STATS n = COUNT(*) BY author.name, `first-name`"
        );
        assert_eq!(params(&query), json!([1, ["x", "y"], "a", "b"]));
        assert_eq!(ident("we`ird"), "`we``ird`");
        assert_eq!(index_pattern("my index"), "\"my index\"");
    }

    #[test]
    fn fork_and_fuse() {
        let query = Esql::from_index("books")
            .metadata(["_id", "_index", "_score"])
            .fork([
                Esql::branch()
                    .where_(col("title").matches("shakespeare"))
                    .sort([col("_score").desc()])
                    .limit(5),
                Esql::branch().where_(col("year").lt(1700)).limit(5),
            ])
            .fuse()
            .keep(["title", "_score"]);
        assert_eq!(
            query.query(),
            "FROM books METADATA _id, _index, _score \
             | FORK (WHERE MATCH(title, ?) | SORT _score DESC | LIMIT 5) (WHERE year < ? | LIMIT 5) \
             | FUSE \
             | KEEP title, _score"
        );
        assert_eq!(params(&query), json!(["shakespeare", 1700]));
        let linear = Esql::branch().fuse_with(
            Fuse::linear()
                .score_by("_score")
                .with(json!({"normalizer": "minmax"})),
        );
        assert_eq!(
            linear.query(),
            r#"FUSE LINEAR SCORE BY _score WITH {"normalizer":"minmax"}"#
        );
    }

    #[test]
    fn requests() {
        let request = Esql::from_index("articles")
            .where_(col("year").eq(2026))
            .request();
        let body = serde_json::to_value(request.body.unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"query": "FROM articles | WHERE year == ?", "params": [2026]})
        );
    }
}
