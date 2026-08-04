use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens, format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, GenericArgument, Ident, Path, PathArguments, Token, Type,
    parenthesized, parse::ParseStream, parse_macro_input, parse_quote, punctuated::Punctuated,
};

struct FieldInfo {
    ident: syn::Ident,
    column: String,
    ty: syn::Type,
    const_ident: syn::Ident,
    is_id: bool,
}

struct EntityAttrs {
    table_name: String,
    comparable: bool,
    hashable: bool,
}

struct ParsedFields {
    fields: Vec<FieldInfo>,
    relationships: Vec<RelationshipDefinition>,
    has_transient: bool,
    has_id: bool,
}

struct RelationshipDefinition {
    field: syn::Ident,
    by_id: bool,
    ty: syn::Type,
    joins: Vec<(Ident, Path)>,
}

struct IndexDefinition<'a> {
    pub columns: Vec<&'a FieldInfo>,
    pub conditions: Vec<(&'a FieldInfo, syn::Lit)>,
    pub unique: bool,
}

fn parse_entity_attrs(input: &DeriveInput, default_table_name: String) -> syn::Result<EntityAttrs> {
    let mut table_name = default_table_name;
    let mut comparable = false;
    let mut hashable = false;

    for attr in &input.attrs {
        if !attr.path().is_ident("entity") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("table") {
                let lit: syn::LitStr = meta.value()?.parse()?;
                table_name = lit.value().trim().to_string();
                if table_name.is_empty() {
                    return Err(meta.error("Attribute table cannot be empty"));
                }
                Ok(())
            } else if meta.path.is_ident("comparable") {
                let lit: syn::LitBool = meta.value()?.parse()?;
                comparable = lit.value();
                Ok(())
            } else if meta.path.is_ident("hashable") {
                let lit: syn::LitBool = meta.value()?.parse()?;
                hashable = lit.value();
                Ok(())
            } else {
                Err(meta.error(
                    "Attribute `entity` not recognized, expected `table = \"...\"`, `comparable = true|false` or `hashable = true|false`",
                ))
            }
        })?;
    }

    Ok(EntityAttrs {
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

fn parse_fields(named_fields: &Punctuated<syn::Field, Token![,]>) -> syn::Result<ParsedFields> {
    let mut has_transient = false;
    let mut has_id = false;
    let mut fields = Vec::new();
    let mut relationships = Vec::new();

    for f in named_fields.iter() {
        let transient = f.attrs.iter().any(|attr| attr.path().is_ident("transient"));
        if transient {
            has_transient = true;
            continue;
        }

        let ident = f.ident.clone().unwrap();
        let name = ident.to_string();
        let const_ident = format_ident!("{}", name.to_uppercase());
        let is_id = f
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("primary_key"));
        has_id = has_id || is_id;
        let mut column_name = name.to_lowercase();
        let mut add = true;

        for attr in &f.attrs {
            if attr.path().is_ident("column") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("name") {
                        let lit: syn::LitStr = meta.value()?.parse()?;
                        column_name = lit.value().trim().to_string();
                        if column_name.is_empty() {
                            return Err(meta.error("Attribute name cannot be empty"));
                        }
                        Ok(())
                    } else {
                        Err(meta
                            .error("Attribute `column` not recognized, expected `name = \"...\"`"))
                    }
                })?;
            } else if attr.path().is_ident("relationship") {
                has_transient = true;
                add = false;

                if let Type::Path(type_path) = &f.ty {
                    // tomamos el ÚLTIMO segmento: soporta Option<T>, std::option::Option<T>, etc.
                    if let Some(segment) = type_path.path.segments.last() {
                        let ident_str = segment.ident.to_string();

                        if let PathArguments::AngleBracketed(args) = &segment.arguments
                            && let Some(GenericArgument::Type(inner_ty)) = args.args.first()
                        {
                            let by_id = match ident_str.as_str() {
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
                                    let pairs = Punctuated::<(Ident, Path), Token![,]>::parse_terminated_with(
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

                            relationships.push({
                                RelationshipDefinition {
                                    field: f.ident.clone().unwrap().clone(),
                                    by_id,
                                    ty: inner_ty.clone(),
                                    joins,
                                }
                            });
                        }
                    }
                }

                continue;
            }
        }

        if add {
            fields.push(FieldInfo {
                ident,
                column: column_name,
                ty: f.ty.clone(),
                const_ident,
                is_id,
            });
        }
    }

    Ok(ParsedFields {
        fields,
        has_transient,
        has_id,
        relationships,
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
    let mut indexes = Vec::new();

    for attr in &input.attrs {
        if !attr.path().is_ident("indexes") && !attr.path().is_ident("uniques") {
            continue;
        }

        let unique = attr.path().is_ident("uniques");

        attr.parse_args_with(|input: ParseStream| {
            while !input.is_empty() {
                let content;
                parenthesized!(content in input);

                let mut columns = Vec::new();
                let mut conditions = Vec::new();

                while !content.is_empty() {
                    let ident = content.parse::<syn::Ident>()?;
                    let field = fields.iter().find(|f| f.ident == ident).ok_or_else(|| {
                        syn::Error::new_spanned(&ident, format!("missing field {}", ident))
                    })?;

                    if content.peek(Token![=]) {
                        content.parse::<Token![=]>()?;
                        let expr: syn::Lit = content.parse()?;
                        conditions.push((field, expr));
                    } else {
                        columns.push(field);
                    }

                    if !content.is_empty() {
                        content.parse::<Token![,]>()?;
                    }
                }

                indexes.push(IndexDefinition {
                    columns,
                    conditions,
                    unique,
                });

                if !input.is_empty() {
                    input.parse::<Token![,]>()?;
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
                rusqlite_orm::dao::helpers::types::where_clause::Where::Eq(self::entity::columns::#const_ident, #value)
            }
        });
        quote! {
            rusqlite_orm::dao::helpers::types::where_clause::Where::And(vec![
                #(#cond),*
            ])
        }
    } else {
        let f = id_fields[0];
        let const_ident = &f.const_ident;
        let value = value_for(f);
        quote! {
            rusqlite_orm::dao::helpers::types::where_clause::Where::Eq(
                self::entity::columns::#const_ident, #value
            )
        }
    }
}

fn build_index_condition(
    index: &IndexDefinition,
    value_for: impl Fn(&FieldInfo) -> TokenStream2,
) -> TokenStream2 {
    if index.columns.len() + index.conditions.len() > 1 {
        let mut cond = Vec::new();
        index.columns.iter().map(|f| {
            let const_ident = &f.const_ident;
            let value = value_for(f);
            quote! {
                rusqlite_orm::dao::helpers::types::where_clause::Where::Eq(self::entity::columns::#const_ident, #value)
            }
        }).for_each(|t|cond.push(t));
        index.conditions.iter().map(|c| {
            let const_ident = &c.0.const_ident;
            let val = c.1.clone();
            let value = quote!{#val.into()};
            quote! {
                rusqlite_orm::dao::helpers::types::where_clause::Where::Eq(self::entity::columns::#const_ident, #value)
            }
        }).for_each(|t|cond.push(t));
        quote! {
            rusqlite_orm::dao::helpers::types::where_clause::Where::And(vec![
                #(#cond),*
            ])
        }
    } else {
        let const_ident;
        let value;
        if let Some(column) = index.columns.first() {
            const_ident = &column.const_ident;
            value = value_for(column);
        } else {
            let condition = index.conditions.first().unwrap();
            const_ident = &condition.0.const_ident;
            let val = condition.1.clone();
            value = quote! {#val.into()};
        }

        quote! {
            rusqlite_orm::dao::helpers::types::where_clause::Where::Eq(
                self::entity::columns::#const_ident, #value
            )
        }
    }
}

fn param_type(f: &FieldInfo) -> TokenStream2 {
    let ident = &f.ident;
    let mut ty = &f.ty;
    let str_ty: Type = parse_quote!(&str);

    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
        && segment.ident == "String"
    {
        ty = &str_ty;
    }
    quote! { #ident: #ty }
}

fn build_entity_module(
    struct_name: &syn::Ident,
    table_name: &str,
    fields: &[FieldInfo],
) -> TokenStream2 {
    let doc = format!("Constant for name of database table {}", table_name);
    let table_name_constant = quote! {
        #[doc = #doc]
        pub const TABLE: rusqlite_orm::dao::helpers::types::table_name::TableName<super::#struct_name> =
            rusqlite_orm::dao::helpers::types::table_name::TableName::<super::#struct_name>::new(#table_name);
    };

    let field_constants = fields.iter().map(|f| {
        let const_ident = &f.const_ident;
        let name = &f.column;
        let doc = format!("Constant for column {} associated to {} field", name, f.ident);
        quote! {
            #[doc = #doc]
            pub const #const_ident: rusqlite_orm::dao::helpers::types::column_name::ColumnName<super::super::#struct_name> =
                rusqlite_orm::dao::helpers::types::column_name::ColumnName::<super::super::#struct_name>::new(#name);
        }
    });

    quote! {
        ///Module with entity table metadata
        pub mod entity {
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
    has_transient: bool,
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

    let default_spread = if has_transient {
        quote! { , ..Default::default() }
    } else {
        quote! {}
    };

    let get_values_lines = fields.iter().map(|f| {
        let ident = &f.ident;
        quote! { self.#ident.clone().into() }
    });
    let repository = format_ident!("{}Repository", struct_name);

    quote! {
        impl rusqlite_orm::dao::Entity for #struct_name {
            #[doc = "Table name constant"]
            const TABLE_NAME: &'static rusqlite_orm::dao::helpers::types::table_name::TableName<Self> = &self::entity::TABLE;

            #[doc = "Array of column names"]
            const FIELDS: &'static [rusqlite_orm::dao::helpers::types::column_name::ColumnName<Self>] =
                &[ #(#field_name_list),* ];

            #[doc = "Repository type"]
            type Repository = #repository;

            #[doc = "Map from resultset row to entity"]
            fn map_from_row(row: &rusqlite_orm::rusqlite::Row) -> Result<Self, rusqlite_orm::rusqlite::Error> {
                Ok(Self {
                    #(#map_from_rows_lines),*
                    #default_spread
                })
            }

            #[doc = "Get array of values from instance"]
            fn get_values(&self) -> Vec<rusqlite_orm::dao::helpers::types::value::Value> {
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
            let condition = if rel.joins.len() == 1 {
                let col = rel.joins.first().unwrap().1.clone();
                let ffj = rel.joins.first().unwrap().0.clone();
                quote! {
                    rusqlite_orm::dao::helpers::types::where_clause::Where::Eq(#col, self.#ffj.clone().into())
                }
            } else {
                let mut cond = Vec::new();
                for join in &rel.joins {
                    let col = join.1.clone();
                    let ffj = join.0.clone();
                    cond.push(quote! {
                        rusqlite_orm::dao::helpers::types::where_clause::Where::Eq(#col, self.#ffj.clone().into())
                    });
                }
                quote! {
                    rusqlite_orm::dao::helpers::types::where_clause::Where::And(vec![
                        #(#cond),*
                    ])
                }
            };
            let fn_inv = if rel.by_id {
                quote! {
                    fetch_one_in_tx
                }
            } else {
                quote! {
                    fetch_in_tx
                }
            };

            let fn_name = format_ident!("fetch_{}_relationship", field);
            let fn_name_tx = format_ident!("fetch_{}_relationship_in_tx", field);

            let doc = format!("Load relationship for {} field", field);
            let doc_tx = format!("{} in tx", doc);

            quote! {
                #[doc = #doc]
                pub fn #fn_name(&mut self) -> rusqlite_orm::database::errors::Result<()>{
                    let mut db = rusqlite_orm::database::DATABASE_INST.lock().unwrap();
                    db.run_in_tx(|tx| self.#fn_name_tx(tx))
                }

                #[doc = #doc_tx]
                pub fn #fn_name_tx(
                    &mut self,
                    tx: &rusqlite_orm::rusqlite::Transaction,
                ) -> rusqlite_orm::database::errors::Result<()>{
                    self.#field = <<#ty as rusqlite_orm::dao::Entity>::Repository as rusqlite_orm::dao::Repository<#ty>>::select().
                    where_(#condition)
                    .#fn_inv(tx)?;
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
            pub fn update_by_id(&self) -> rusqlite_orm::database::errors::Result<()> {
                let mut db = rusqlite_orm::database::DATABASE_INST.lock().unwrap();
                db.run_in_tx(|tx| self.update_by_id_in_tx(tx))
            }

            #[doc = "Update row by primary key in transaction"]
            pub fn update_by_id_in_tx(
                &self,
                tx: &rusqlite_orm::rusqlite::Transaction,
            ) -> rusqlite_orm::database::errors::Result<()> {
                <#repo_ident as rusqlite_orm::dao::Repository<#struct_name>>::update()
                    #(#update_sets)*
                    .where_(#update_delete_condition)
                    .execute_in_tx(tx)?;
                Ok(())
            }

            #[doc = "Delete row by primary key"]
            pub fn delete_by_id(&self) -> rusqlite_orm::database::errors::Result<()> {
                let mut db = rusqlite_orm::database::DATABASE_INST.lock().unwrap();
                db.run_in_tx(|tx| self.delete_by_id_in_tx(tx))
            }

            #[doc = "Delete row by primary key in transaction"]
            pub fn delete_by_id_in_tx(
                &self,
                tx: &rusqlite_orm::rusqlite::Transaction,
            ) -> rusqlite_orm::database::errors::Result<()> {
                <#repo_ident as rusqlite_orm::dao::Repository<#struct_name>>::delete()
                    .where_(#update_delete_condition)
                    .execute_in_tx(tx)?;
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
                #(#by_id_params),*
            ) -> rusqlite_orm::database::errors::Result<bool> {
                let mut db = rusqlite_orm::database::DATABASE_INST.lock().unwrap();
                db.run_in_tx(|tx| Self::exists_in_tx(tx, #id_condition_names))
            }
            #[doc = "Checks if row exists in transaction"]
            pub fn exists_in_tx(
                tx: &rusqlite_orm::rusqlite::Transaction,
                #(#by_id_params),*
            ) -> rusqlite_orm::database::errors::Result<bool> {
                let count = <Self as rusqlite_orm::dao::Repository<#struct_name>>::select()
                    .where_(#id_condition)
                    .count_in_tx(tx)?;
                Ok(count>0)
            }

            #[doc = "Fetch row by primary key"]
            pub fn select_by_id(
                #(#by_id_params),*
            ) -> rusqlite_orm::database::errors::Result<Option<#struct_name>> {
                let mut db = rusqlite_orm::database::DATABASE_INST.lock().unwrap();
                db.run_in_tx(|tx| Self::select_by_id_in_tx(tx, #id_condition_names))
            }

            #[doc = "Fetch row by primary key in transaction"]
            pub fn select_by_id_in_tx(
                tx: &rusqlite_orm::rusqlite::Transaction,
                #(#by_id_params),*
            ) -> rusqlite_orm::database::errors::Result<Option<#struct_name>> {
                Ok(<Self as rusqlite_orm::dao::Repository<#struct_name>>::select()
                    .where_(#id_condition)
                    .fetch_in_tx(tx)?
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
        let mut fn_name = format_ident!(
            "select_by_{}",
            &index.columns.iter().map(|i| i.ident.to_string()).collect::<Vec<String>>().join("_and_"),
        );
        let mut fn_name_count = format_ident!(
            "count_by_{}",
            &index.columns.iter().map(|i| i.ident.to_string()).collect::<Vec<String>>().join("_and_"),
        );

        if !index.conditions.is_empty(){
            let sufix = format!("where_{}",
                index.conditions.iter().map(|c|{
                    format!("{}_eq_{}",c.0.ident, c.1.to_token_stream())
                }).collect::<Vec<String>>().join("_and_").replace("\"","").replace("'","").replace(" ","_")
            );
            fn_name = format_ident!("{}_{}",fn_name, sufix);
            fn_name_count = format_ident!("{}_{}",fn_name_count, sufix);
        }

        let fn_name_tx = format_ident!(
            "{}_in_tx",
            fn_name,
        );
        let fn_name_count_tx = format_ident!(
            "{}_in_tx",
            fn_name_count,
        );

        let select_params: Vec<TokenStream2> = index.columns.iter().map(|f| param_type(f)).collect();

        let condition = build_index_condition(index, |f| {
            let ident = &f.ident;
            quote! { #ident.into() }
        });

        let repo_ident = format_ident!("{}Repository", struct_name);

        let  idx_col_names= index.columns.iter().map(|i| i.ident.to_string()).collect::<Vec<String>>().join(", ");
        let idx_col_names_idents = index.columns.iter().map(|i| i.ident.clone()).collect::<Vec<syn::Ident>>();
        let doc1=format!("Fetch row{} by {} index", if index.unique {""} else {"s"}, idx_col_names);
        let doc2=format!("Fetch row{} by {} index in transaction", if index.unique {""} else {"s"}, idx_col_names);
        let doc3=format!("Count row{} by {} index", if index.unique {""} else {"s"}, idx_col_names);
        let doc4=format!("Count row{} by {} index in transaction", if index.unique {""} else {"s"}, idx_col_names);

        let order_by_arg = if index.unique {
            quote! {}
        } else {
            quote!{
                , order_by: Option<&[rusqlite_orm::dao::helpers::types::order_by::OrderBy<#struct_name>]>
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
            quote!{fetch_one_in_tx(tx)}
        } else {
            quote! {fetch_in_tx(tx)}
        };

        quote! {
            #[doc = #doc1]
            pub fn #fn_name(#(#select_params),* #order_by_arg) -> rusqlite_orm::database::errors::Result<#ret_type> {
                let mut db = rusqlite_orm::database::DATABASE_INST.lock().unwrap();
                db.run_in_tx(|tx| Self::#fn_name_tx(tx, #(#idx_col_names_idents),* #order_by_param))
            }

            #[doc = #doc2]
            pub fn #fn_name_tx(tx: &rusqlite_orm::rusqlite::Transaction, #(#select_params),* #order_by_arg) -> rusqlite_orm::database::errors::Result<#ret_type> {
                let mut builder = <#repo_ident as rusqlite_orm::dao::Repository<#struct_name>>::select()
                    .where_(#condition);
                #add_order_by
                builder.#fetch_inv
            }

            #[doc = #doc3]
            pub fn #fn_name_count(#(#select_params),*) -> rusqlite_orm::database::errors::Result<i64> {
                let mut db = rusqlite_orm::database::DATABASE_INST.lock().unwrap();
                db.run_in_tx(|tx| Self::#fn_name_count_tx(tx, #(#idx_col_names_idents),*))
            }

            #[doc = #doc4]
            pub fn #fn_name_count_tx(tx: &rusqlite_orm::rusqlite::Transaction, #(#select_params),*) -> rusqlite_orm::database::errors::Result<i64> {
                <#repo_ident as rusqlite_orm::dao::Repository<#struct_name>>::select()
                    .where_(#condition).count_in_tx(tx)
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
        has_transient,
        has_id,
        relationships,
    } = bail_on_err!(parse_fields(named_fields));

    let repo_ident = format_ident!("{}Repository", struct_name);

    bail_on_err!(validate_id_requirements(
        &input,
        has_id,
        entity_attrs.comparable,
        entity_attrs.hashable
    ));

    let indexes = bail_on_err!(parse_indexes(&input, &fields));
    let id_fields: Vec<&FieldInfo> = fields.iter().filter(|f| f.is_id).collect();

    let entity_module = build_entity_module(struct_name, &entity_attrs.table_name, &fields);
    let entity_trait_impl = build_entity_trait_impl(struct_name, &fields, has_transient);
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
