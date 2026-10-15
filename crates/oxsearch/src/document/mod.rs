//! The typed document layer: a struct is a document type, its fields define
//! the index's mapping, and an [`Index`] reads, writes and searches it.
//!
//! ```ignore
//! use oxsearch::document::{Document, Indexes};
//! use oxsearch::Retriever;
//!
//! #[derive(Document, Serialize, Deserialize)]
//! #[index("articles")]
//! struct Article {
//!     #[id]
//!     id: String,
//!     #[semantic_text(inference_id = ".elser-2-elasticsearch")]
//!     body: String,
//!     #[keyword]
//!     tags: Vec<String>,
//!     #[dense_vector(dims = 384, similarity = "cosine")]
//!     embedding: Vec<f32>,
//! }
//!
//! let articles = client.index::<Article>();
//! articles.ensure_mapping().await?;
//! articles.put(&article).await?;
//! let hits = articles
//!     .search(Retriever::rrf([
//!         Retriever::standard(Article::BODY.semantic("how do I tune HNSW?")),
//!         Article::EMBEDDING.knn(embed("how do I tune HNSW?"), 10, 100),
//!     ]))
//!     .await?;
//! ```
//!
//! Field attributes choose a field's mapped type: `#[keyword]`, `#[text]`,
//! `#[date]`, `#[semantic_text(inference_id = "…")]`,
//! `#[dense_vector(dims = …, similarity = "…", element_type = "…")]` and
//! `#[sparse_vector]`. Without one, the Rust type decides through
//! [`Mapped`]: strings are `text`, numbers and booleans their own types,
//! date and time types `date`, and a struct that derives `Document` an
//! object. `#[id]` and `#[routing]` mark the fields that give a document's
//! id and routing. serde's `rename`, `rename_all` and `skip` are followed.

mod field;
mod index;

pub use field::{Field, kind};
pub use index::{Drift, Index, Indexes, MappingReport, Version, Versioned, Written};
#[cfg(feature = "derive")]
pub use oxsearch_derive::Document;

use oxsearch_core::shapes::IndexMap;
use oxsearch_mapping::{
    BooleanProperty, ByteNumberProperty, DoubleNumberProperty, FloatNumberProperty,
    IntegerNumberProperty, LongNumberProperty, Property, ShortNumberProperty, TextProperty,
    TypeMapping, UnsignedLongNumberProperty,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A document type: the mapping its fields define, and how to find a
/// document's id and routing. Derive it with `#[derive(Document)]`.
pub trait Document: Serialize + DeserializeOwned {
    /// The index `#[index("…")]` names, if any.
    const INDEX: Option<&'static str>;

    /// The mapping's properties, one for each field.
    fn properties() -> IndexMap<String, Property>;

    /// The mapping.
    fn mapping() -> TypeMapping {
        TypeMapping::new().properties(Self::properties())
    }

    /// The document's id, from its `#[id]` field. Without one,
    /// Elasticsearch gives each new document an id.
    fn id(&self) -> Option<String> {
        None
    }

    /// The document's routing, from its `#[routing]` field.
    fn routing(&self) -> Option<String> {
        None
    }
}

/// A Rust type's default mapped type, for fields without an attribute that
/// chooses one.
pub trait Mapped {
    /// What a [`Field`] of this type offers: a marker from [`kind`].
    type Kind;

    /// The field's mapping.
    fn property() -> Property;
}

macro_rules! mapped {
    ($kind:ident: $($ty:ty => $property:expr),* $(,)?) => {
        $(
            impl Mapped for $ty {
                type Kind = kind::$kind;
                fn property() -> Property {
                    $property
                }
            }
        )*
    };
}

mapped!(Text: String => Property::Text(TextProperty::new()));
mapped!(Boolean: bool => Property::Boolean(BooleanProperty::new()));
mapped!(Number:
    i8 => Property::ByteNumber(ByteNumberProperty::new()),
    i16 => Property::ShortNumber(ShortNumberProperty::new()),
    i32 => Property::IntegerNumber(IntegerNumberProperty::new()),
    i64 => Property::LongNumber(LongNumberProperty::new()),
    u8 => Property::ShortNumber(ShortNumberProperty::new()),
    u16 => Property::IntegerNumber(IntegerNumberProperty::new()),
    u32 => Property::LongNumber(LongNumberProperty::new()),
    u64 => Property::UnsignedLongNumber(UnsignedLongNumberProperty::new()),
    f32 => Property::FloatNumber(FloatNumberProperty::new()),
    f64 => Property::DoubleNumber(DoubleNumberProperty::new()),
);

/// Elasticsearch has no array type: a field holds one value or many.
macro_rules! many {
    ($($ty:ty),* $(,)?) => {
        $(
            impl<T: Mapped> Mapped for $ty {
                type Kind = T::Kind;
                fn property() -> Property {
                    T::property()
                }
            }
        )*
    };
}

many!(
    Option<T>,
    Vec<T>,
    Box<T>,
    std::collections::VecDeque<T>,
    std::collections::BTreeSet<T>,
    std::collections::HashSet<T>,
);

impl<T: Mapped, const N: usize> Mapped for [T; N] {
    type Kind = T::Kind;
    fn property() -> Property {
        T::property()
    }
}

/// What the derive macro's code uses, so it compiles whatever the user's
/// crate imports.
#[doc(hidden)]
pub mod __private {
    pub use oxsearch_core::shapes::IndexMap;
    pub use oxsearch_mapping::{
        DateProperty, DenseVectorElementType, DenseVectorProperty, DenseVectorSimilarity,
        KeywordProperty, ObjectProperty, Property, SemanticTextProperty, SparseVectorProperty,
        TextProperty,
    };

    pub use super::field::kind;
    pub use super::{Document, Field, Mapped};

    /// A field's value as an id or a routing value.
    pub fn text(value: &impl ToString) -> String {
        value.to_string()
    }
}
