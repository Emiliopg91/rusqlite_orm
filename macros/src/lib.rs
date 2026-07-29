use std::fs;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, LitStr, Token, Type, parenthesized, parse::ParseStream,
    parse_macro_input, parse_quote, punctuated::Punctuated,
};

#[proc_macro]
pub fn dlls(input: TokenStream) -> TokenStream {
    let lit = parse_macro_input!(input as LitStr);

    let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(d) => d,
        Err(_) => {
            return syn::Error::new(lit.span(), "CARGO_MANIFEST_DIR not set")
                .to_compile_error()
                .into();
        }
    };
    let ddls_dir = std::path::Path::new(&manifest_dir)
        .join(lit.value())
        .display()
        .to_string();

    let read_dir = match fs::read_dir(&ddls_dir) {
        Ok(rd) => rd,
        Err(e) => {
            return syn::Error::new(
                lit.span(),
                format!("could not read directory `{ddls_dir}`: {e}"),
            )
            .to_compile_error()
            .into();
        }
    };

    let files = read_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "sql").unwrap_or(false))
        .collect::<Vec<_>>();

    let mut entries = Vec::with_capacity(files.len());

    for path in &files {
        let name = match path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => {
                return syn::Error::new(
                    lit.span(),
                    format!("invalid file path: {}", path.display()),
                )
                .to_compile_error()
                .into();
            }
        };

        let version = match name.split('_').next().and_then(|v| v.parse::<u16>().ok()) {
            Some(v) => v,
            None => {
                return syn::Error::new(
                    lit.span(),
                    format!(
                        "invalid DDL file name `{name}`: expected format `<version>_<name>.sql`"
                    ),
                )
                .to_compile_error()
                .into();
            }
        };

        let mut content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                return syn::Error::new(
                    lit.span(),
                    format!("could not read `{}`: {e}", path.display()),
                )
                .to_compile_error()
                .into();
            }
        };

        let description = match content
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("--"))
        {
            Some(d) => d.trim().to_string(),
            None => {
                return syn::Error::new(
                    lit.span(),
                    format!("missing description comment in `{name}`"),
                )
                .to_compile_error()
                .into();
            }
        };

        content = content
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                if l.is_empty() || l.starts_with("--") {
                    None
                } else {
                    Some(l.to_string())
                }
            })
            .collect::<Vec<String>>()
            .join("\n");

        entries.push(quote! {
            rusqlite_orm::database::DdlVersion {
                version: #version,
                description: #description,
                sql: #content,
            }
        });
    }

    let len = entries.len();

    quote! {
        pub static DDLS: [rusqlite_orm::database::DdlVersion; #len] = [
            #(#entries),*
        ];
    }
    .into()
}

#[proc_macro_derive(Entity, attributes(entity, primary_key, dont_map, column, indexes))]
pub fn derive_entity(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;
    let mut comparable = false;
    let mut hashable = false;

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

    let mut table_name = struct_name.to_string().to_lowercase();
    for attr in &input.attrs {
        if !attr.path().is_ident("entity") {
            continue;
        }

        let mut found_table = false;
        let result = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("table") {
                let lit: syn::LitStr = meta.value()?.parse()?;
                table_name = lit.value().trim().to_string();
                if table_name.is_empty() {
                    return Err(meta.error("Attribute table cannot be empty"));
                }
                found_table = true;
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
        });

        if let Err(err) = result {
            return err.to_compile_error().into();
        }
    }

    let named_fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            _ => {
                return syn::Error::new_spanned(
                    &input,
                    "Entity only can be derived in named structs",
                )
                .to_compile_error()
                .into();
            }
        },
        _ => {
            return syn::Error::new_spanned(&input, "Entity only can be derived in structs")
                .to_compile_error()
                .into();
        }
    };

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
                let result = attr.parse_nested_meta(|meta| {
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
                });

                if let Err(err) = result {
                    return err.to_compile_error().into();
                }
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

    if !has_id {
        if comparable {
            return syn::Error::new_spanned(input, "comparable requires id columns")
                .to_compile_error()
                .into();
        }
        if hashable {
            return syn::Error::new_spanned(input, "hashable requires id columns")
                .to_compile_error()
                .into();
        }
    }

    let mut indexes = Vec::new();

    for attr in &input.attrs {
        if attr.path().is_ident("indexes") {
            let result = attr.parse_args_with(|input: ParseStream| {
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
            });

            match result {
                Ok(groups) => {
                    for group in groups {
                        let mut index = Vec::new();
                        let cols: Vec<syn::Ident> = group.iter().cloned().collect();

                        for col in cols {
                            let field = fields.iter().find(|f| f.ident == col);
                            match field {
                                Some(field) => index.push(field),
                                None => {
                                    return syn::Error::new_spanned(
                                        attr,
                                        format!("missing field {}", col),
                                    )
                                    .to_compile_error()
                                    .into();
                                }
                            }
                        }

                        indexes.push(index);
                    }
                }
                Err(err) => return err.to_compile_error().into(),
            }
        }
    }

    let mut indexes_expand = Vec::new();
    for index in indexes {
        let fn_name = format_ident!(
            "select_by_{}",
            index
                .iter()
                .map(|i| i.ident.to_string())
                .collect::<Vec<String>>()
                .join("_and_"),
        );
        let fn_name_tx = format_ident!(
            "select_by_{}_in_tx",
            index
                .iter()
                .map(|i| i.ident.to_string())
                .collect::<Vec<String>>()
                .join("_and_"),
        );
        let select_params: Vec<TokenStream2> = index
            .iter()
            .map(|f| {
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
            })
            .collect();

        let condition = build_condition(&index, |f| {
            let ident = &f.ident;
            quote! { #ident.into() }
        });

        let expand = quote! {
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
        };

        indexes_expand.push(expand);
    }

    let indexes_impl = if indexes_expand.is_empty() {
        quote! {}
    } else {
        quote! {
            impl #struct_name {
                #(#indexes_expand)*
            }
        }
    };

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

    let field_name_list = fields.iter().map(|f| {
        let ident = &f.const_ident;
        quote! {
            self::entity::columns::#ident
        }
    });

    let map_from_rows_lines = fields.iter().enumerate().map(|(idx, f)| {
        let ident = &f.ident;
        let ty = &f.ty;
        quote! {
            #ident: row.get::<_, #ty>(#idx)?
        }
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

    let id_fields: Vec<&FieldInfo> = fields.iter().filter(|f| f.is_id).collect();
    let primary_key_operation = if id_fields.is_empty() {
        quote! {}
    } else {
        let by_id_params: Vec<TokenStream2> = id_fields
            .iter()
            .map(|f| {
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
            })
            .collect();

        let id_condition = build_condition(&id_fields, |f| {
            let ident = &f.ident;
            quote! { #ident.into() }
        });

        let update_delete_condition = build_condition(&id_fields, |f| {
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
            impl #struct_name {
                pub fn select_by_id(
                    #(#by_id_params),*
                ) -> rusqlite_orm::database::errors::Result<Option<Self>> {
                    Ok(<#struct_name as rusqlite_orm::dao::Entity>::select()
                        .where_(#id_condition)
                        .fetch()?
                        .into_iter()
                        .next())
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
                    <#struct_name as rusqlite_orm::dao::Entity>::update()
                        #(#update_sets)*
                        .where_(#update_delete_condition)
                        .execute()?;
                    Ok(())
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
                    <#struct_name as rusqlite_orm::dao::Entity>::delete()
                        .where_(#update_delete_condition)
                        .execute()?;
                    Ok(())
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
    };

    let comparable_impl = if !comparable {
        quote! {}
    } else {
        let comparaisons = id_fields
            .iter()
            .map(|field| {
                let name = &field.ident;
                quote! {
                    self.#name == other.#name
                }
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
    };

    let hashable_impl = if !hashable {
        quote! {}
    } else {
        let hashes = id_fields
            .iter()
            .map(|field| {
                let name = &field.ident;
                quote! {
                    self.#name.hash(state);
                }
            })
            .collect::<Vec<TokenStream2>>();

        quote! {
            impl std::hash::Hash for #struct_name {
                fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                    #(#hashes)*
                }
            }
        }
    };

    let expanded = quote! {
        pub mod entity {
            #table_name_constant
            pub mod columns {
                #(#field_constants)*
            }
        }

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

        #comparable_impl

        #hashable_impl

        #primary_key_operation

        #indexes_impl
    };

    expanded.into()
}

struct FieldInfo {
    ident: syn::Ident,
    column: String,
    ty: syn::Type,
    const_ident: syn::Ident,
    is_id: bool,
}
