use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, Token, Type, parenthesized, parse::ParseStream, parse_macro_input,
    parse_quote, punctuated::Punctuated,
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
    has_dont_map: bool,
    has_id: bool,
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
    let mut has_dont_map = false;
    let mut has_id = false;
    let mut fields: Vec<FieldInfo> = Vec::new();

    for f in named_fields.iter() {
        let dont_map = f.attrs.iter().any(|attr| attr.path().is_ident("dont_map"));
        if dont_map {
            has_dont_map = true;
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
            }
        }

        fields.push(FieldInfo {
            ident,
            column: column_name,
            ty: f.ty.clone(),
            const_ident,
            is_id,
        });
    }

    Ok(ParsedFields {
        fields,
        has_dont_map,
        has_id,
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
) -> syn::Result<Vec<Vec<&'a FieldInfo>>> {
    let mut indexes = Vec::new();

    for attr in &input.attrs {
        if !attr.path().is_ident("indexes") {
            continue;
        }

        let groups: Vec<Punctuated<syn::Ident, Token![,]>> =
            attr.parse_args_with(|input: ParseStream| {
                let mut groups: Vec<Punctuated<syn::Ident, Token![,]>> = Vec::new();

                while !input.is_empty() {
                    let content;
                    parenthesized!(content in input);
                    let idents = Punctuated::<syn::Ident, Token![,]>::parse_terminated(&content)?;
                    groups.push(idents);

                    if !input.is_empty() {
                        input.parse::<Token![,]>()?;
                    }
                }

                Ok(groups)
            })?;

        for group in groups {
            let mut index = Vec::new();
            for col in group.iter() {
                match fields.iter().find(|f| &f.ident == col) {
                    Some(field) => index.push(field),
                    None => {
                        return Err(syn::Error::new_spanned(
                            attr,
                            format!("missing field {}", col),
                        ));
                    }
                }
            }
            indexes.push(index);
        }
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
    let table_name_constant = quote! {
        pub const TABLE: rusqlite_orm::dao::helpers::types::table_name::TableName<super::#struct_name> =
            rusqlite_orm::dao::helpers::types::table_name::TableName::<super::#struct_name>::new(#table_name);
    };

    let field_constants = fields.iter().map(|f| {
        let const_ident = &f.const_ident;
        let name = &f.column;
        quote! {
            pub const #const_ident: rusqlite_orm::dao::helpers::types::column_name::ColumnName<super::super::#struct_name> =
                rusqlite_orm::dao::helpers::types::column_name::ColumnName::<super::super::#struct_name>::new(#name);
        }
    });

    quote! {
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
    has_dont_map: bool,
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

    let default_spread = if has_dont_map {
        quote! { , ..Default::default() }
    } else {
        quote! {}
    };

    let get_values_lines = fields.iter().map(|f| {
        let ident = &f.ident;
        quote! { self.#ident.clone().into() }
    });

    quote! {
        impl rusqlite_orm::dao::Entity for #struct_name {
            const TABLE_NAME: &'static rusqlite_orm::dao::helpers::types::table_name::TableName<Self> = &self::entity::TABLE;
            const FIELDS: &'static [rusqlite_orm::dao::helpers::types::column_name::ColumnName<Self>] =
                &[ #(#field_name_list),* ];

            fn map_from_row(row: &rusqlite_orm::rusqlite::Row) -> Result<Self, rusqlite_orm::rusqlite::Error> {
                Ok(Self {
                    #(#map_from_rows_lines),*
                    #default_spread
                })
            }

            fn get_values(&self) -> Vec<rusqlite_orm::dao::helpers::types::value::Value> {
                vec![
                    #(#get_values_lines),*
                ]
            }
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

    let by_id_params: Vec<TokenStream2> = id_fields.iter().map(|f| param_type(f)).collect();

    let id_condition = build_condition(id_fields, |f| {
        let ident = &f.ident;
        quote! { #ident.into() }
    });

    let id_condition_idents = id_fields.iter().map(|f| f.ident.clone());
    let id_condition_names = quote! { #(#id_condition_idents),* };

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

    quote! {
            pub fn exists(
                #(#by_id_params),*
            ) -> rusqlite_orm::database::errors::Result<bool> {
                let mut db = rusqlite_orm::database::DATABASE_INST.lock().unwrap();
                db.run_in_tx(|tx| Self::exists_in_tx(tx, #id_condition_names))
            }
            pub fn exists_in_tx(
                tx: &rusqlite_orm::rusqlite::Transaction,
                #(#by_id_params),*
            ) -> rusqlite_orm::database::errors::Result<bool> {
                let count = <#struct_name as rusqlite_orm::dao::Entity>::select()
                    .where_(#id_condition)
                    .count_in_tx(tx)?;
                Ok(count>0)
            }

            pub fn select_by_id(
                #(#by_id_params),*
            ) -> rusqlite_orm::database::errors::Result<Option<Self>> {
                let mut db = rusqlite_orm::database::DATABASE_INST.lock().unwrap();
                db.run_in_tx(|tx| Self::select_by_id_in_tx(tx, #id_condition_names))
            }

            pub fn select_by_id_in_tx(
                tx: &rusqlite_orm::rusqlite::Transaction,
                #(#by_id_params),*
            ) -> rusqlite_orm::database::errors::Result<Option<Self>> {
                Ok(<#struct_name as rusqlite_orm::dao::Entity>::select()
                    .where_(#id_condition)
                    .fetch_in_tx(tx)?
                    .into_iter()
                    .next())
            }

            pub fn update_by_id(&self) -> rusqlite_orm::database::errors::Result<()> {
                let mut db = rusqlite_orm::database::DATABASE_INST.lock().unwrap();
                db.run_in_tx(|tx| self.update_by_id_in_tx(tx))
            }

            pub fn update_by_id_in_tx(
                &self,
                tx: &rusqlite_orm::rusqlite::Transaction,
            ) -> rusqlite_orm::database::errors::Result<()> {
                <#struct_name as rusqlite_orm::dao::Entity>::update()
                    #(#update_sets)*
                    .where_(#update_delete_condition)
                    .execute_in_tx(tx)?;
                Ok(())
            }

            pub fn delete_by_id(&self) -> rusqlite_orm::database::errors::Result<()> {
                let mut db = rusqlite_orm::database::DATABASE_INST.lock().unwrap();
                db.run_in_tx(|tx| self.delete_by_id_in_tx(tx))
            }

            pub fn delete_by_id_in_tx(
                &self,
                tx: &rusqlite_orm::rusqlite::Transaction,
            ) -> rusqlite_orm::database::errors::Result<()> {
                <#struct_name as rusqlite_orm::dao::Entity>::delete()
                    .where_(#update_delete_condition)
                    .execute_in_tx(tx)?;
                Ok(())
            }
    }
}

fn build_indexes_impl(struct_name: &syn::Ident, indexes: &[Vec<&FieldInfo>]) -> TokenStream2 {
    if indexes.is_empty() {
        return quote! {};
    }

    let indexes_expand = indexes.iter().map(|index| {
        let fn_name = format_ident!(
            "select_by_{}",
            index.iter().map(|i| i.ident.to_string()).collect::<Vec<String>>().join("_and_"),
        );
        let fn_name_tx = format_ident!(
            "select_by_{}_in_tx",
            index.iter().map(|i| i.ident.to_string()).collect::<Vec<String>>().join("_and_"),
        );
        let fn_name_count = format_ident!(
            "count_by_{}",
            index.iter().map(|i| i.ident.to_string()).collect::<Vec<String>>().join("_and_"),
        );
        let fn_name_count_tx = format_ident!(
            "count_by_{}_in_tx",
            index.iter().map(|i| i.ident.to_string()).collect::<Vec<String>>().join("_and_"),
        );
        let select_params: Vec<TokenStream2> = index.iter().map(|f| param_type(f)).collect();

        let condition = build_condition(index, |f| {
            let ident = &f.ident;
            quote! { #ident.into() }
        });

        quote! {
            pub fn #fn_name(#(#select_params),*, order_by: Option<&[rusqlite_orm::dao::helpers::types::order_by::OrderBy<Self>]>) -> rusqlite_orm::database::errors::Result<Vec<Self>> {
                let mut builder = <#struct_name as rusqlite_orm::dao::Entity>::select()
                    .where_(#condition);
                if let Some(order_by) = order_by{
                    for ob in order_by {
                        builder = builder.order_by((*ob).clone());
                    }
                }
                builder.fetch()
            }
            pub fn #fn_name_tx(tx: &rusqlite_orm::rusqlite::Transaction, #(#select_params),*, order_by: Option<&[rusqlite_orm::dao::helpers::types::order_by::OrderBy<Self>]>) -> rusqlite_orm::database::errors::Result<Vec<Self>> {
                let mut builder = <#struct_name as rusqlite_orm::dao::Entity>::select()
                    .where_(#condition);
                if let Some(order_by) = order_by{
                    for ob in order_by {
                        builder = builder.order_by((*ob).clone());
                    }
                }
                builder.fetch_in_tx(tx)
            }
            pub fn #fn_name_count(#(#select_params),*) -> rusqlite_orm::database::errors::Result<i64> {
                <#struct_name as rusqlite_orm::dao::Entity>::select()
                    .where_(#condition).count()
            }
            pub fn #fn_name_count_tx(tx: &rusqlite_orm::rusqlite::Transaction, #(#select_params),*) -> rusqlite_orm::database::errors::Result<i64> {
                <#struct_name as rusqlite_orm::dao::Entity>::select()
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
        has_dont_map,
        has_id,
    } = bail_on_err!(parse_fields(named_fields));

    bail_on_err!(validate_id_requirements(
        &input,
        has_id,
        entity_attrs.comparable,
        entity_attrs.hashable
    ));

    let indexes = bail_on_err!(parse_indexes(&input, &fields));
    let id_fields: Vec<&FieldInfo> = fields.iter().filter(|f| f.is_id).collect();

    let entity_module = build_entity_module(struct_name, &entity_attrs.table_name, &fields);
    let entity_trait_impl = build_entity_trait_impl(struct_name, &fields, has_dont_map);
    let primary_key_operation = build_primary_key_impl(struct_name, &fields, &id_fields);
    let indexes_impl = build_indexes_impl(struct_name, &indexes);
    let comparable_impl = build_comparable_impl(struct_name, entity_attrs.comparable, &id_fields);
    let hashable_impl = build_hashable_impl(struct_name, entity_attrs.hashable, &id_fields);

    let expanded = quote! {
        #entity_module

        #entity_trait_impl

        #comparable_impl

        #hashable_impl

        impl #struct_name {
            #indexes_impl
            #primary_key_operation
        }
    };

    expanded.into()
}
