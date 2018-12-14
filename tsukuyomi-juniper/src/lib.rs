//! Components for integrating GraphQL endpoints into Tsukuyomi.

#![doc(html_root_url = "https://docs.rs/tsukuyomi-juniper/0.3.0-dev")]
#![deny(
    missing_docs,
    missing_debug_implementations,
    nonstandard_style,
    rust_2018_idioms,
    rust_2018_compatibility,
    unused
)]
#![forbid(clippy::unimplemented)]

mod error;
mod graphiql;
mod request;

pub use crate::{
    error::GraphQLModifier,
    graphiql::graphiql_source,
    request::{request, GraphQLRequest, GraphQLResponse},
};

use {
    juniper::{GraphQLType, RootNode},
    std::sync::Arc,
};

/// A marker trait representing a root node of GraphQL schema.
#[allow(missing_docs)]
pub trait Schema {
    type Query: GraphQLType<Context = Self::Context, TypeInfo = Self::QueryInfo>;
    type QueryInfo;
    type Mutation: GraphQLType<Context = Self::Context, TypeInfo = Self::MutationInfo>;
    type MutationInfo;
    type Context;

    fn as_root_node(&self) -> &RootNode<'static, Self::Query, Self::Mutation>;
}

impl<QueryT, MutationT, CtxT> Schema for RootNode<'static, QueryT, MutationT>
where
    QueryT: GraphQLType<Context = CtxT>,
    MutationT: GraphQLType<Context = CtxT>,
{
    type Query = QueryT;
    type QueryInfo = QueryT::TypeInfo;
    type Mutation = MutationT;
    type MutationInfo = MutationT::TypeInfo;
    type Context = CtxT;

    #[inline]
    fn as_root_node(&self) -> &RootNode<'static, Self::Query, Self::Mutation> {
        self
    }
}

impl<S> Schema for Box<S>
where
    S: Schema,
{
    type Query = S::Query;
    type QueryInfo = S::QueryInfo;
    type Mutation = S::Mutation;
    type MutationInfo = S::MutationInfo;
    type Context = S::Context;

    #[inline]
    fn as_root_node(&self) -> &RootNode<'static, Self::Query, Self::Mutation> {
        (**self).as_root_node()
    }
}

impl<S> Schema for Arc<S>
where
    S: Schema,
{
    type Query = S::Query;
    type QueryInfo = S::QueryInfo;
    type Mutation = S::Mutation;
    type MutationInfo = S::MutationInfo;
    type Context = S::Context;

    #[inline]
    fn as_root_node(&self) -> &RootNode<'static, Self::Query, Self::Mutation> {
        (**self).as_root_node()
    }
}
