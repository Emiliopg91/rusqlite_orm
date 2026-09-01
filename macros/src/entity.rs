use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, GenericArgument, Ident, Path, PathArguments, Token, Type,
    parenthesized, parse::ParseStream, parse_macro_input, parse_quote, punctuated::Punctuated,
};

pub fn derive_entity(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;

    macro_rules! bail_on_err {
        ($expr:expr) => {
            match $expr {
                Ok(value) => value,
                Err(err) => return err.to_compile_error().into(),
            }
        };
    }

    let default_table_name = struct_name.to_string().to_lowercase();
    let entity_attrs = bail_on_err!(parse_entity_attrs(&input, default_table_name));

    let named_fields = bail_on_err!(get_named_fields(&input));
    let ParsedFields {
        fields,
        has_id,
        relationships,
        transients,
    } = bail_on_err!(parse_fields(&input, named_fields));

    let repo_ident = format_ident!("{}Repository", struct_name);

    bail_on_err!(validate_id_requirements(
        &input,
        has_id,
        entity_attrs.comparable,
        entity_attrs.hashable
    ));

    let indexes = bail_on_err!(parse_indexes(&input, &fields));
    let id_fields: Vec<&FieldInfo> = fields.iter().filter(|f| f.is_id).collect();

    let entity_module = build_entity_module(
        struct_name,
        &entity_attrs.schema_name,
        &entity_attrs.table_name,
        &fields,
    );
    let entity_trait_impl = build_entity_trait_impl(struct_name, &fields, transients);
    let primary_key_operation = build_primary_key_impl(struct_name, &fields, &id_fields);
    let repository_primary_key_operation =
        repository_build_primary_key_impl(struct_name, &id_fields);
    let indexes_impl = build_indexes_impl(struct_name, &indexes);
    let comparable_impl = build_comparable_impl(struct_name, entity_attrs.comparable, &id_fields);
    let hashable_impl = build_hashable_impl(struct_name, entity_attrs.hashable, &id_fields);
    let relationships_impl = build_entity_with_relationships_trait_impl(relationships);

    let repo_doc = format!("Repository for {}", struct_name);
    let expanded = quote! {
        #entity_module

        #entity_trait_impl

        #comparable_impl

        #hashable_impl

        impl #struct_name {
            #primary_key_operation
            #relationships_impl
        }

        #[doc = #repo_doc]
        pub struct #repo_ident;
        impl rusqlite_orm::dao::Repository<#struct_name> for #repo_ident{}
        impl #repo_ident{
            #indexes_impl
            #repository_primary_key_operation
        }
    };

    expanded.into()
}

fn parse_entity_attrs(input: &DeriveInput, default_table_name: String) -> syn::Result<EntityAttrs> {
    let mut schema_name = "main".to_string();
    let mut table_name = default_table_name;
    let mut comparable = false;
    let mut hashable = false;
    let mut already_found = false;

    for attr in &input.attrs {
        if !attr.path().is_ident("entity") {
            continue;
        }

        if already_found {
            return Err(syn::Error::new_spanned(
                attr,
                "Duplicated attribute 'entity'",
            ));
        }

        already_found = true;

        if let Ok(table_name_lit) = attr.parse_args::<syn::LitStr>() {
            table_name = table_name_lit.value();
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("schema") {
                let lit = meta.value()?.parse::<syn::LitStr>()?;
                schema_name = lit.value().trim().to_string();
                if schema_name.is_empty() {
                    return Err(meta.error("Attribute schema cannot be empty"));
                }
                Ok(())
            } else if meta.path.is_ident("table") {
                let lit = meta.value()?.parse::<syn::LitStr>()?;
                table_name = lit.value().trim().to_string();
                if table_name.is_empty() {
                    return Err(meta.error("Attribute table cannot be empty"));
                }
                Ok(())
            } else if meta.path.is_ident("comparable") {
                let lit = meta.value()?.parse::<syn::LitBool>()?;
                comparable = lit.value();
                Ok(())
            } else if meta.path.is_ident("hashable") {
                let lit = meta.value()?.parse::<syn::LitBool>()?;
                hashable = lit.value();
                Ok(())
            } else {
                Err(meta.error(
                    "Attribute `entity` not recognized, expected `schema = \"...\"`, `table = \"...\"`, `comparable = true|false` or `hashable = true|false`",
                ))
            }
        })?;
    }

    Ok(EntityAttrs {
        schema_name,
        table_name,
        comparable,
        hashable,
    })
}

fn get_named_fields(input: &DeriveInput) -> syn::Result<&Punctuated<syn::Field, Token![,]>> {
    match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => Ok(&named.named),
            _ => Err(syn::Error::new_spanned(
                input,
                "Entity only can be derived in named structs",
            )),
        },
        _ => Err(syn::Error::new_spanned(
            input,
            "Entity only can be derived in structs",
        )),
    }
}

fn parse_fields(
    input: &DeriveInput,
    named_fields: &Punctuated<syn::Field, Token![,]>,
) -> syn::Result<ParsedFields> {
    let mut has_id = false;
    let mut fields: Vec<FieldInfo> = Vec::new();
    let mut relationships = Vec::new();
    let mut transients = Vec::new();
    let mut id_fields = Vec::new();

    for attr in &input.attrs {
        if attr.path().is_ident("primary_key") {
            attr.parse_args_with(|input: ParseStream| {
                while !input.is_empty() {
                    let id_field = input.parse::<syn::Ident>()?;
                    id_fields.push(id_field);

                    if input.peek(Token![,]) {
                        input.parse::<Token![,]>()?;
                    }
                }

                Ok(())
            })?;

            break;
        }
    }

    for f in named_fields.iter() {
        if f.attrs.iter().any(|attr| attr.path().is_ident("transient"))
            && f.attrs
                .iter()
                .any(|attr| attr.path().is_ident("relationship"))
        {
            return Err(syn::Error::new_spanned(
                f.attrs
                    .iter()
                    .find(|a| a.path().is_ident("relationship"))
                    .unwrap(),
                "Incompatible attributes transient and relationship",
            ));
        }

        if f.attrs.iter().any(|attr| attr.path().is_ident("transient")) {
            transients.push(f.ident.clone().unwrap());
            continue;
        }

        let ident = f.ident.clone().unwrap();
        let name = ident.to_string();
        let const_ident = format_ident!("{}", name.to_uppercase());
        let is_id = if let Some(idx) = id_fields.iter().position(|x| *x == ident) {
            id_fields.remove(idx);
            true
        } else {
            false
        };
        has_id = has_id || is_id;
        let mut column_name = name.to_lowercase();
        let mut add = true;

        for attr in &f.attrs {
            if attr.path().is_ident("column") {
                let lit = attr.parse_args::<syn::LitStr>()?;
                column_name = lit.value().trim().to_string();
                if column_name.is_empty() {
                    return Err(syn::Error::new_spanned(
                        attr,
                        "Attribute name cannot be empty",
                    ));
                }
            } else if attr.path().is_ident("relationship") {
                add = false;
                transients.push(f.ident.clone().unwrap());

                let Type::Path(type_path) = &f.ty else {
                    return Err(syn::Error::new_spanned(f, "Expected Option or Vec"));
                };

                let Some(segment) = type_path.path.segments.last() else {
                    return Err(syn::Error::new_spanned(
                        f,
                        "Cannot determine relationship type",
                    ));
                };

                let PathArguments::AngleBracketed(args) = &segment.arguments else {
                    return Err(syn::Error::new_spanned(f, "Expected Option or Vec"));
                };

                let Some(generic_arg) = args.args.first() else {
                    return Err(syn::Error::new_spanned(f, "Cannot extract generic type"));
                };

                let GenericArgument::Type(inner_ty) = generic_arg else {
                    return Err(syn::Error::new_spanned(
                        f,
                        "Expected type in generic argument",
                    ));
                };

                let by_id = match segment.ident.to_string().as_str() {
                    "Option" => true,
                    "Vec" => false,
                    _ => {
                        return Err(syn::Error::new_spanned(
                            f,
                            "Relationship only can be holded in Vec or Option",
                        ));
                    }
                };

                let joins: Vec<(Ident, Path)> = attr.parse_args_with(|input: ParseStream| {
                    let pairs: Punctuated<(Ident, Path), syn::token::Comma> =
                        Punctuated::<(Ident, Path), Token![,]>::parse_terminated_with(
                            input,
                            |input: ParseStream| {
                                let content;
                                syn::parenthesized!(content in input);

                                let local_field: Ident = content.parse()?;
                                content.parse::<Token![,]>()?;
                                let remote_column: Path = content.parse()?;

                                Ok((local_field, remote_column))
                            },
                        )?;
                    Ok(pairs.into_iter().collect())
                })?;

                if joins.is_empty() {
                    return Err(syn::Error::new_spanned(
                        attr,
                        "Columns for join cannot be empty",
                    ));
                }

                relationships.push(RelationshipDefinition {
                    field: f.ident.clone().unwrap(),
                    by_id,
                    ty: inner_ty.clone(),
                    joins,
                });

                continue;
            }
        }

        if add {
            if fields.iter().any(|f| f.column == column_name) {
                return Err(syn::Error::new_spanned(f, "Duplicated column name"));
            }

            fields.push(FieldInfo {
                ident,
                column: column_name,
                ty: f.ty.clone(),
                const_ident,
                is_id,
            });
        }
    }

    if let Some(unknown_field) = id_fields.first() {
        return Err(syn::Error::new_spanned(
            unknown_field,
            "Field not found in struct",
        ));
    }

    Ok(ParsedFields {
        fields,
        has_id,
        relationships,
        transients,
    })
}

fn validate_id_requirements(
    input: &DeriveInput,
    has_id: bool,
    comparable: bool,
    hashable: bool,
) -> syn::Result<()> {
    if !has_id {
        if comparable {
            return Err(syn::Error::new_spanned(
                input,
                "comparable requires id columns",
            ));
        }
        if hashable {
            return Err(syn::Error::new_spanned(
                input,
                "hashable requires id columns",
            ));
        }
    }
    Ok(())
}

fn parse_indexes<'a>(
    input: &DeriveInput,
    fields: &'a [FieldInfo],
) -> syn::Result<Vec<IndexDefinition<'a>>> {
    let mut indexes: Vec<IndexDefinition<'_>> = Vec::new();

    for attr in &input.attrs {
        if !attr.path().is_ident("index") && !attr.path().is_ident("unique") {
            continue;
        }

        let unique = attr.path().is_ident("unique");

        attr.parse_args_with(|input: ParseStream| {
            match input.parse::<syn::LitStr>() {
                Err(_) => {
                    return Err(syn::Error::new_spanned(
                        attr,
                        "First argument must be a string literal for name",
                    ));
                }
                Ok(name_lit) => {
                    let name = name_lit.value();

                    if indexes.iter().any(|i| i.name == name) {
                        return Err(syn::Error::new_spanned(attr, "Duplicated index name"));
                    }

                    input.parse::<Token![,]>()?;

                    let mut columns: Vec<&FieldInfo> = Vec::new();

                    let content;
                    parenthesized!(content in input);
                    while !content.is_empty() {
                        let ident = content.parse::<syn::Ident>()?;
                        let field = fields.iter().find(|f| f.ident == ident).ok_or_else(|| {
                            syn::Error::new_spanned(&ident, format!("missing field {}", ident))
                        })?;
                        if columns.iter().any(|c| c.ident == field.ident) {
                            return Err(syn::Error::new_spanned(
                                ident,
                                "Duplicated column in index",
                            ));
                        }

                        columns.push(field);

                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                    if columns.is_empty() {
                        return Err(content.error("Index columns cannot be empty"));
                    }

                    let mut conditions = None;

                    if input.parse::<Token![,]>().is_ok() {
                        let content;
                        parenthesized!(content in input);
                        let mut conditions_vec = Vec::new();
                        while !content.is_empty() {
                            let ident = content.parse::<syn::Ident>()?;
                            let field =
                                fields.iter().find(|f| f.ident == ident).ok_or_else(|| {
                                    syn::Error::new_spanned(
                                        &ident,
                                        format!("missing field {}", ident),
                                    )
                                })?;

                            if content.peek(Token![=]) {
                                content.parse::<Token![=]>()?;
                                let expr: syn::Lit = content.parse()?;
                                conditions_vec.push((field, expr));
                            } else {
                                return Err(content.error("Missing = "));
                            }

                            if !content.is_empty() {
                                content.parse::<Token![,]>()?;
                            }
                        }

                        if conditions_vec.is_empty() {
                            return Err(content.error("Conditions defined empty"));
                        }

                        conditions = Some(conditions_vec);
                    }

                    indexes.push(IndexDefinition {
                        name,
                        columns,
                        conditions,
                        unique,
                    });
                }
            }
            Ok(())
        })?;
    }

    Ok(indexes)
}

fn build_condition(
    id_fields: &[&FieldInfo],
    value_for: impl Fn(&FieldInfo) -> TokenStream2,
) -> TokenStream2 {
    if id_fields.len() > 1 {
        let cond = id_fields.iter().map(|f| {
            let const_ident = &f.const_ident;
            let value = value_for(f);
            quote! {
                rusqlite_orm::types::where_clause::Where::Eq(self::entity::columns::#const_ident, #value)
            }
        });
        quote! {
            rusqlite_orm::types::where_clause::Where::And(vec![
                #(#cond),*
            ])
        }
    } else {
        let f = id_fields[0];
        let const_ident = &f.const_ident;
        let value = value_for(f);
        quote! {
            rusqlite_orm::types::where_clause::Where::Eq(
                self::entity::columns::#const_ident, #value
            )
        }
    }
}

fn build_index_condition(
    index: &IndexDefinition,
    value_for: impl Fn(&FieldInfo) -> TokenStream2,
) -> TokenStream2 {
    if index.columns.len() + index.conditions.as_ref().map_or(0, |v| v.len()) > 1 {
        let mut cond = Vec::new();
        index.columns.iter().map(|f| {
            let const_ident = &f.const_ident;
            let value = value_for(f);
            quote! {
                rusqlite_orm::types::where_clause::Where::Eq(self::entity::columns::#const_ident, #value)
            }
        }).for_each(|t|cond.push(t));
        if let Some(conditions) = &index.conditions {
            conditions.iter().map(|c| {
                let const_ident = &c.0.const_ident;
                let val = c.1.clone();
                let value = quote!{#val.into()};
                quote! {
                    rusqlite_orm::types::where_clause::Where::Eq(self::entity::columns::#const_ident, #value)
                }
            }).for_each(|t|cond.push(t));
        }
        quote! {
            rusqlite_orm::types::where_clause::Where::And(vec![
                #(#cond),*
            ])
        }
    } else {
        let const_ident;
        let value;
        if let Some(conditions) = &index.conditions
            && !conditions.is_empty()
        {
            let condition = conditions.first().unwrap();
            const_ident = &condition.0.const_ident;
            let val = condition.1.clone();
            value = quote! {#val.into()};
        } else {
            let column = index.columns.first().unwrap();
            const_ident = &column.const_ident;
            value = value_for(column);
        }

        quote! {
            rusqlite_orm::types::where_clause::Where::Eq(
                self::entity::columns::#const_ident, #value
            )
        }
    }
}

fn param_type(f: &FieldInfo) -> TokenStream2 {
    let ident = &f.ident;
    let mut ty = &f.ty;
    let str_ty: Type = parse_quote!(&str);
    let bytes_ty: Type = parse_quote!(&[u8]);

    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
    {
        ty = match segment.ident.to_string().as_ref() {
            "String" => &str_ty,
            "Vec" => match &segment.arguments {
                PathArguments::AngleBracketed(args) => match args.args.first() {
                    Some(GenericArgument::Type(Type::Path(inner))) if inner.path.is_ident("u8") => {
                        &bytes_ty
                    }
                    _ => ty,
                },
                _ => ty,
            },
            _ => ty,
        }
    }
    quote! { #ident: #ty }
}

fn build_entity_module(
    struct_name: &syn::Ident,
    schema_name: &str,
    table_name: &str,
    fields: &[FieldInfo],
) -> TokenStream2 {
    let doc = format!("Constant for name of database schema {}", table_name);
    let schema_name_constant = quote! {
        #[doc = #doc]
        pub const SCHEMA: rusqlite_orm::types::schema::Schema<super::#struct_name> =
            rusqlite_orm::types::schema::Schema::<super::#struct_name>::new(#schema_name);
    };

    let doc = format!("Constant for name of database table {}", table_name);
    let table_name_constant = quote! {
        #[doc = #doc]
        pub const TABLE: rusqlite_orm::types::table_name::TableName<super::#struct_name> =
            rusqlite_orm::types::table_name::TableName::<super::#struct_name>::new(#table_name);
    };

    let field_constants = fields.iter().map(|f| {
        let const_ident = &f.const_ident;
        let name = &f.column;
        let doc = format!("Constant for column {} associated to {} field", name, f.ident);
        quote! {
            #[doc = #doc]
            pub const #const_ident: rusqlite_orm::types::column_name::ColumnName<super::super::#struct_name> =
                rusqlite_orm::types::column_name::ColumnName::<super::super::#struct_name>::new(#name);
        }
    });

    quote! {
        ///Module with entity table metadata
        pub mod entity {
            #schema_name_constant
            #table_name_constant
            pub mod columns {
                #(#field_constants)*
            }
        }
    }
}

fn build_entity_trait_impl(
    struct_name: &syn::Ident,
    fields: &[FieldInfo],
    transients: Vec<Ident>,
) -> TokenStream2 {
    let field_name_list = fields.iter().map(|f| {
        let ident = &f.const_ident;
        quote! { self::entity::columns::#ident }
    });

    let map_from_rows_lines = fields.iter().enumerate().map(|(idx, f)| {
        let ident = &f.ident;
        let ty = &f.ty;
        quote! { #ident: row.get::<_, #ty>(#idx)? }
    });

    let default_spread = transients.iter().map(|i| {
        quote! {
            , #i: Default::default()
        }
    });

    let get_values_lines = fields.iter().map(|f| {
        let ident = &f.ident;
        quote! { self.#ident.clone().into() }
    });
    let repository = format_ident!("{}Repository", struct_name);

    quote! {
        impl rusqlite_orm::dao::Entity for #struct_name {
            #[doc = "Database schema constant"]
            const SCHEMA: &'static rusqlite_orm::types::schema::Schema<Self> = &self::entity::SCHEMA;

            #[doc = "Table name constant"]
            const TABLE_NAME: &'static rusqlite_orm::types::table_name::TableName<Self> = &self::entity::TABLE;

            #[doc = "Array of column names"]
            const FIELDS: &'static [rusqlite_orm::types::column_name::ColumnName<Self>] =
                &[ #(#field_name_list),* ];

            #[doc = "Repository type"]
            type Repository = #repository;

            #[doc = "Map from resultset row to entity"]
            fn map_from_row(row: &rusqlite_orm::rusqlite::Row) -> Result<Self, rusqlite_orm::rusqlite::Error> {
                Ok(Self {
                    #(#map_from_rows_lines),*
                    #(#default_spread)*
                })
            }

            #[doc = "Get array of values from instance"]
            fn get_values(&self) -> Vec<rusqlite_orm::types::value::Value> {
                vec![
                    #(#get_values_lines),*
                ]
            }
        }
    }
}

fn build_entity_with_relationships_trait_impl(
    relationships: Vec<RelationshipDefinition>,
) -> TokenStream2 {
    if relationships.is_empty() {
        quote! {}
    } else {
        let impls = relationships.iter().map(|rel|  {
            let field = &rel.field;
            let ty = &rel.ty;
            // Un solo join => igualdad simple; varios joins => Where::And de igualdades.
            let condition = if rel.joins.len() == 1 {
                let col = rel.joins.first().unwrap().1.clone();
                let ffj = rel.joins.first().unwrap().0.clone();
                quote! {
                    rusqlite_orm::types::where_clause::Where::Eq(#col, self.#ffj.clone().into())
                }
            } else {
                let mut cond = Vec::new();
                for join in &rel.joins {
                    let col = join.1.clone();
                    let ffj = join.0.clone();
                    cond.push(quote! {
                        rusqlite_orm::types::where_clause::Where::Eq(#col, self.#ffj.clone().into())
                    });
                }
                quote! {
                    rusqlite_orm::types::where_clause::Where::And(vec![
                        #(#cond),*
                    ])
                }
            };
            // Relación Option<T> => se espera como mucho una fila (fetch_one); Vec<T> => varias.
            let fn_inv = if rel.by_id {
                quote! {
                    fetch_one_in
                }
            } else {
                quote! {
                    fetch_in
                }
            };

            let fn_name = format_ident!("fetch_{}_relationship", field);
            let fn_name_conn = format_ident!("fetch_{}_relationship_in_conn", field);

            let doc = format!("Load relationship for {} field", field);
            let doc_conn = format!("{} in conn", doc);

            quote! {
                #[doc = #doc]
                pub fn #fn_name(&mut self,db: &rusqlite_orm::database::DatabaseConnection) -> rusqlite_orm::errors::Result<()>{
                    db.run_in_connection(|conn| {
                        let res = self.#fn_name_conn(conn)?;
                        Ok(res)
                    })
                }

                #[doc = #doc_conn]
                pub fn #fn_name_conn(&mut self, conn: &rusqlite_orm::rusqlite::Connection) -> rusqlite_orm::errors::Result<()>{
                    self.#field = <<#ty as rusqlite_orm::dao::Entity>::Repository as rusqlite_orm::dao::Repository<#ty>>::select().
                    where_(#condition)
                    .#fn_inv(conn)?;
                    Ok(())
                }
            }
        });

        quote! {
            #(#impls)*
        }
    }
}

fn build_primary_key_impl(
    struct_name: &syn::Ident,
    fields: &[FieldInfo],
    id_fields: &[&FieldInfo],
) -> TokenStream2 {
    if id_fields.is_empty() {
        return quote! {};
    }

    let update_delete_condition = build_condition(id_fields, |f| {
        let ident = &f.ident;
        quote! { self.#ident.clone().into() }
    });

    let update_sets: Vec<TokenStream2> = fields
        .iter()
        .filter(|f| !f.is_id)
        .map(|f| {
            let ident = &f.ident;
            let const_ident = &f.const_ident;
            quote! {
                .set(self::entity::columns::#const_ident, self.#ident.clone().into())
            }
        })
        .collect();

    let repo_ident = format_ident!("{}Repository", struct_name);

    quote! {
            #[doc = "Update row by primary key"]
            pub fn update_by_id(&self,db: &rusqlite_orm::database::DatabaseConnection, ) -> rusqlite_orm::errors::Result<()> {
                db.run_in_transaction(|tx| {
                    let res = self.update_by_id_in(tx)?;
                    Ok(res)
                })
            }

            #[doc = "Update row by primary key in connection"]
            pub fn update_by_id_in(&self, tx: &rusqlite_orm::rusqlite::Transaction) -> rusqlite_orm::errors::Result<()> {
                <#repo_ident as rusqlite_orm::dao::Repository<#struct_name>>::update()
                    #(#update_sets)*
                    .where_(#update_delete_condition)
                    .execute_in(tx)?;
                Ok(())
            }

            #[doc = "Delete row by primary key"]
            pub fn delete_by_id(&self, db: &rusqlite_orm::database::DatabaseConnection) -> rusqlite_orm::errors::Result<()> {
                db.run_in_transaction(|tx| {
                    let res = self.delete_by_id_in(tx)?;
                    Ok(res)
                })
            }

            #[doc = "Delete row by primary key in connection"]
            pub fn delete_by_id_in(&self, tx: &rusqlite_orm::rusqlite::Transaction) -> rusqlite_orm::errors::Result<()> {
                <#repo_ident as rusqlite_orm::dao::Repository<#struct_name>>::delete()
                    .where_(#update_delete_condition)
                    .execute_in(tx)?;
                Ok(())
            }
    }
}

fn repository_build_primary_key_impl(
    struct_name: &syn::Ident,
    id_fields: &[&FieldInfo],
) -> TokenStream2 {
    if id_fields.is_empty() {
        return quote! {};
    }

    let by_id_params: Vec<TokenStream2> = id_fields.iter().map(|f| param_type(f)).collect();

    let id_condition = build_condition(id_fields, |f| {
        let ident = &f.ident;
        quote! { #ident.into() }
    });

    let id_condition_idents = id_fields.iter().map(|f| f.ident.clone());
    let id_condition_names = quote! { #(#id_condition_idents),* };

    quote! {
            #[doc = "Checks if row exists"]
            pub fn exists(
                db: &rusqlite_orm::database::DatabaseConnection, 
                #(#by_id_params),*
            ) -> rusqlite_orm::errors::Result<bool> {
                db.run_in_connection(|conn| {
                    let res = Self::exists_in(conn, #id_condition_names)?;
                    Ok(res)
                })
            }
            #[doc = "Checks if row exists in connection"]
            pub fn exists_in(conn: &rusqlite_orm::rusqlite::Connection,
                #(#by_id_params),*
            ) -> rusqlite_orm::errors::Result<bool> {
                let count = <Self as rusqlite_orm::dao::Repository<#struct_name>>::select()
                    .where_(#id_condition)
                    .count_in(conn)?;
                Ok(count>0)
            }

            #[doc = "Fetch row by primary key"]
            pub fn select_by_id(
                db: &rusqlite_orm::database::DatabaseConnection, 
                #(#by_id_params),*
            ) -> rusqlite_orm::errors::Result<Option<#struct_name>> {
                db.run_in_connection(|conn| {
                    let res = Self::select_by_id_in(conn, #id_condition_names)?;
                    Ok(res)
                })
            }

            #[doc = "Fetch row by primary key in connection"]
            pub fn select_by_id_in(
                conn: &rusqlite_orm::rusqlite::Connection,
                #(#by_id_params),*
            ) -> rusqlite_orm::errors::Result<Option<#struct_name>> {
                Ok(<Self as rusqlite_orm::dao::Repository<#struct_name>>::select()
                    .where_(#id_condition)
                    .fetch_in(conn)?
                    .into_iter()
                    .next())
            }
    }
}

fn build_indexes_impl(struct_name: &syn::Ident, indexes: &[IndexDefinition]) -> TokenStream2 {
    if indexes.is_empty() {
        return quote! {};
    }

    let indexes_expand = indexes.iter().map(|index| {
        let fn_name = format_ident!(
            "select_by_{}",
            index.name,
        );
        let fn_name_conn = format_ident!(
            "{}_in_conn",
            fn_name,
        );
        let fn_name_count = format_ident!(
            "{}_by_{}",
            if index.unique {"exists"} else {"count"},
            index.name,
        );
        let fn_name_count_conn = format_ident!(
            "{}_in_conn",
            fn_name_count,
        );

        let select_params: Vec<TokenStream2> = index.columns.iter().map(|f| param_type(f)).collect();

        let condition = build_index_condition(index, |f| {
            let ident = &f.ident;
            quote! { #ident.into() }
        });

        let repo_ident = format_ident!("{}Repository", struct_name);

        let idx_col_names_idents = index.columns.iter().map(|i| i.ident.clone()).collect::<Vec<syn::Ident>>();
        let doc1=format!("Fetch row{} by {} index", if index.unique {""} else {"s"}, index.name);
        let doc3=format!("{} by {} index", if index.unique {"Exists"} else {"Count rows"}, index.name);

        let order_by_arg = if index.unique {
            quote! {}
        } else {
            quote!{
                , order_by: Option<&[rusqlite_orm::types::order_by::OrderBy<#struct_name>]>
            }
        };

        let ret_type = if index.unique {
            quote! {
                Option<#struct_name>
            }
        } else {
            quote!{
                Vec<#struct_name>
            }
        };

        let cnt_ret_type = if index.unique {
            quote! {
                bool
            }
        } else {
            quote!{
                i64
            }
        };

        let cnt_impl = if index.unique{
            quote!{
                Ok(<#repo_ident as rusqlite_orm::dao::Repository<#struct_name>>::select()
                    .where_(#condition).count_in(conn)?>0)
            }
        } else {
            quote! {
                <#repo_ident as rusqlite_orm::dao::Repository<#struct_name>>::select()
                    .where_(#condition).count_in(conn)
            }
        };

        let add_order_by = if index.unique{
            quote! {}
        } else {
            quote!{
                if let Some(order_by) = order_by{
                    for ob in order_by {
                        builder = builder.order_by((*ob).clone());
                    }
                }
            }
        };
        let order_by_param = if index.unique {
            quote!{}
        } else {
            quote! {, order_by}
        };
        let fetch_inv = if index.unique {
            quote!{fetch_one_in(conn)}
        } else {
            quote! {fetch_in(conn)}
        };

        quote! {
            #[doc = #doc1]
            pub fn #fn_name(db: &rusqlite_orm::database::DatabaseConnection, #(#select_params),* #order_by_arg) -> rusqlite_orm::errors::Result<#ret_type> {
                db.run_in_connection(|conn| {
                    let res = Self::#fn_name_conn(conn, #(#idx_col_names_idents),* #order_by_param)?;
                    Ok(res)
                })
            }
            #[doc = #doc1]
            pub fn #fn_name_conn(conn: &rusqlite_orm::rusqlite::Connection, #(#select_params),* #order_by_arg) -> rusqlite_orm::errors::Result<#ret_type> {
                let mut builder = <#repo_ident as rusqlite_orm::dao::Repository<#struct_name>>::select()
                    .where_(#condition);
                #add_order_by
                builder.#fetch_inv
            }


            #[doc = #doc3]
            pub fn #fn_name_count(db: &rusqlite_orm::database::DatabaseConnection, #(#select_params),*) -> rusqlite_orm::errors::Result<#cnt_ret_type> {
                db.run_in_connection(|conn| {
                    let res = Self::#fn_name_count_conn(conn, #(#idx_col_names_idents),*)?;
                    Ok(res)
                })
            }

            #[doc = #doc3]
            pub fn #fn_name_count_conn(conn: &rusqlite_orm::rusqlite::Connection, #(#select_params),*) -> rusqlite_orm::errors::Result<#cnt_ret_type> {
                #cnt_impl
            }
        }
    });

    quote! {
        #(#indexes_expand)*
    }
}

fn build_comparable_impl(
    struct_name: &syn::Ident,
    comparable: bool,
    id_fields: &[&FieldInfo],
) -> TokenStream2 {
    if !comparable {
        return quote! {};
    }

    let comparaisons = id_fields
        .iter()
        .map(|field| {
            let name = &field.ident;
            quote! { self.#name == other.#name }
        })
        .collect::<Vec<TokenStream2>>();

    quote! {
        impl PartialEq for #struct_name {
            fn eq(&self, other: &Self) -> bool {
                #(#comparaisons)&& *
            }
        }
        impl Eq for #struct_name {}
    }
}

fn build_hashable_impl(
    struct_name: &syn::Ident,
    hashable: bool,
    id_fields: &[&FieldInfo],
) -> TokenStream2 {
    if !hashable {
        return quote! {};
    }

    let hashes = id_fields
        .iter()
        .map(|field| {
            let name = &field.ident;
            quote! { self.#name.hash(state); }
        })
        .collect::<Vec<TokenStream2>>();

    quote! {
        impl std::hash::Hash for #struct_name {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                #(#hashes)*
            }
        }
    }
}

struct FieldInfo {
    ident: syn::Ident,
    column: String,
    ty: syn::Type,
    const_ident: syn::Ident,
    is_id: bool,
}

struct EntityAttrs {
    schema_name: String,
    table_name: String,
    comparable: bool,
    hashable: bool,
}

struct ParsedFields {
    fields: Vec<FieldInfo>,
    transients: Vec<Ident>,
    relationships: Vec<RelationshipDefinition>,
    has_id: bool,
}

struct RelationshipDefinition {
    field: syn::Ident,
    by_id: bool,
    ty: syn::Type,
    joins: Vec<(Ident, Path)>,
}

struct IndexDefinition<'a> {
    pub name: String,
    pub columns: Vec<&'a FieldInfo>,
    pub conditions: Option<Vec<(&'a FieldInfo, syn::Lit)>>,
    pub unique: bool,
}
