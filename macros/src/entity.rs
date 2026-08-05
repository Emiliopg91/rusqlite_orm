use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, GenericArgument, Ident, Path, PathArguments, Token, Type,
    parenthesized, parse::ParseStream, parse_macro_input, parse_quote, punctuated::Punctuated,
};


/// Punto de entrada del `#[derive(Entity)]`: orquesta el parseo de atributos/campos/índices y
/// combina la salida de cada `build_*` en el módulo `entity`, el `impl Entity`, los `impl`
/// opcionales (`PartialEq`/`Eq`, `Hash`) y la struct `<Struct>Repository` con sus métodos.
pub fn derive_entity(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;

    // Cualquier `syn::Result::Err` en las funciones de parseo se convierte directamente en un
    // error de compilación con el span del nodo correspondiente, en vez de un `panic!`.
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
        transients
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

    let entity_module = build_entity_module(struct_name, &entity_attrs.schema_name, &entity_attrs.table_name, &fields);
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


/// Lee el atributo `#[entity(table = "...", comparable = bool, hashable = bool)]` de la struct.
/// Cualquier clave distinta de `table`/`comparable`/`hashable` produce un error de compilación.
fn parse_entity_attrs(input: &DeriveInput, default_table_name: String) -> syn::Result<EntityAttrs> {
    let mut schema_name = "main".to_string();
    let mut table_name = default_table_name;
    let mut comparable = false;
    let mut hashable = false;

    for attr in &input.attrs {
        if !attr.path().is_ident("entity") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("schema") {
                let lit: syn::LitStr = meta.value()?.parse()?;
                schema_name = lit.value().trim().to_string();
                if schema_name.is_empty() {
                    return Err(meta.error("Attribute table cannot be empty"));
                }
                Ok(())
            } else if meta.path.is_ident("table") {
                let lit: syn::LitStr = meta.value()?.parse()?;
                table_name = lit.value().trim().to_string();
                if table_name.is_empty() {
                    return Err(meta.error("Attribute schema cannot be empty"));
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

/// Extrae los campos con nombre de la struct de entrada; falla si `derive(Entity)` se aplica
/// sobre algo distinto de una struct con campos nombrados (enums, tuplas, unit structs, etc.).
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

/// Clasifica cada campo de la struct en: transient (ignorado por la persistencia), relationship
/// (produce una `RelationshipDefinition` y no se mapea a columna) o campo normal (produce un
/// `FieldInfo`). También resuelve el nombre de columna a partir de `#[column(name = "...")]`.
fn parse_fields(named_fields: &Punctuated<syn::Field, Token![,]>) -> syn::Result<ParsedFields> {
    let mut has_id = false;
    let mut fields = Vec::new();
    let mut relationships = Vec::new();
    let mut transients = Vec::new();

    for f in named_fields.iter() {
        if f.attrs.iter().any(|attr| attr.path().is_ident("transient")) {
            transients.push(f.ident.clone().unwrap());
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
                let lit: syn::LitStr = attr.parse_args()?;
                column_name = lit.value().trim().to_string();
                if column_name.is_empty() {
                    return Err(syn::Error::new_spanned(attr, "Attribute name cannot be empty"));
                }
            } else if attr.path().is_ident("relationship") {
                // Las relaciones no se persisten como columna propia: se marcan como transient
                // para que map_from_row las rellene con Default y no se incluyan en get_values.
                add = false;
             transients.push(f.ident.clone().unwrap());

                if let Type::Path(type_path) = &f.ty {
                    // tomamos el ÚLTIMO segmento: soporta Option<T>, std::option::Option<T>, etc.
                    if let Some(segment) = type_path.path.segments.last() {
                        let ident_str = segment.ident.to_string();

                        // Solo nos interesa el tipo genérico interno T de Option<T>/Vec<T>.
                        if let PathArguments::AngleBracketed(args) = &segment.arguments
                            && let Some(GenericArgument::Type(inner_ty)) = args.args.first()
                        {
                            // Option<T> => relación a lo sumo 1 (by_id); Vec<T> => relación a muchos.
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

                            // Sintaxis: #[relationship((local_field, remote::Column), (local_field2, remote::Column2), ...)]
                            // Cada par entre paréntesis define una condición de join local = remoto.
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
        has_id,
        relationships,
        transients
    })
}

/// `comparable`/`hashable` dependen de comparar por clave primaria, así que exigen que la
/// entidad tenga al menos un campo `#[primary_key]`.
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

/// Parsea los atributos `#[indexes((col1, col2), (col3 = "literal"), ...)]` y
/// `#[uniques(...)]` de la struct. Dentro de cada grupo entre paréntesis, un identificador
/// suelto es una columna variable del índice (recibida como parámetro en las funciones
/// generadas) y un `ident = literal` es una condición fija (el índice queda restringido a
/// ese valor constante). Puede haber varios grupos de índices, cada uno entre paréntesis.
fn parse_indexes<'a>(
    input: &DeriveInput,
    fields: &'a [FieldInfo],
) -> syn::Result<Vec<IndexDefinition<'a>>> {
    let mut indexes = Vec::new();

    for attr in &input.attrs {
        if !attr.path().is_ident("index") && !attr.path().is_ident("unique") {
            continue;
        }

        let unique = attr.path().is_ident("unique");

        attr.parse_args_with(|input: ParseStream|{
            match input.parse::<syn::LitStr>() {
                Err(_)=> {
                    return Err(syn::Error::new_spanned(attr, "First argument must be a string literal for name"));
                }
                Ok(name_lit) => {
                    let name = name_lit.value();

                    input.parse::<Token![,]>()?;

                    let mut columns = Vec::new();

                    let content ; 
                    parenthesized!(content in input);
                    while !content.is_empty() {
                        let ident = content.parse::<syn::Ident>()?;
                        let field = fields.iter().find(|f| f.ident == ident).ok_or_else(|| {
                            syn::Error::new_spanned(&ident, format!("missing field {}", ident))
                        })?;
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
                        let content ; 
                        parenthesized!(content in input);
                        let mut conditions_vec = Vec::new();
                        while !content.is_empty() {
                            let ident = content.parse::<syn::Ident>()?;
                            let field = fields.iter().find(|f| f.ident == ident).ok_or_else(|| {
                                syn::Error::new_spanned(&ident, format!("missing field {}", ident))
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

/// Genera la expresión `Where` que identifica una fila por su clave primaria: una única
/// igualdad si hay un solo campo id, o un `Where::And` de igualdades si la clave es compuesta.
/// `value_for` decide cómo obtener el valor de cada campo (p. ej. `self.campo` o el parámetro
/// de función homónimo), lo que permite reutilizar esta función tanto en el `impl` de la
/// entidad como en el del repositorio.
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

/// Análogo a `build_condition` pero para un `IndexDefinition`: combina las columnas variables
/// (usando `value_for`) y las condiciones fijas (literal embebido en el propio atributo) en un
/// único `Where`, uniéndolas con `And` cuando hay más de una.
fn build_index_condition(
    index: &IndexDefinition,
    value_for: impl Fn(&FieldInfo) -> TokenStream2,
) -> TokenStream2 {
    if index.columns.len() + index.conditions.clone().unwrap_or_default().len() > 1 {
        let mut cond = Vec::new();
        index.columns.iter().map(|f| {
            let const_ident = &f.const_ident;
            let value = value_for(f);
            quote! {
                rusqlite_orm::dao::helpers::types::where_clause::Where::Eq(self::entity::columns::#const_ident, #value)
            }
        }).for_each(|t|cond.push(t));
        if let Some(conditions) = &index.conditions {
            conditions.iter().map(|c| {
                let const_ident = &c.0.const_ident;
                let val = c.1.clone();
                let value = quote!{#val.into()};
                quote! {
                    rusqlite_orm::dao::helpers::types::where_clause::Where::Eq(self::entity::columns::#const_ident, #value)
                }
            }).for_each(|t|cond.push(t));
        }
        quote! {
            rusqlite_orm::dao::helpers::types::where_clause::Where::And(vec![
                #(#cond),*
            ])
        }
    } else {
        let const_ident;
        let value;
        if let Some(conditions) = &index.conditions && !conditions.is_empty(){
            let condition = conditions.first().unwrap();
            const_ident = &condition.0.const_ident;
            let val = condition.1.clone();
            value = quote! {#val.into()};
        } else {
            let column  =index.columns.first().unwrap();
            const_ident = &column.const_ident;
            value = value_for(column);
        }

        quote! {
            rusqlite_orm::dao::helpers::types::where_clause::Where::Eq(
                self::entity::columns::#const_ident, #value
            )
        }
    }
}

/// Construye el parámetro de función `nombre: Tipo` para un campo, sustituyendo `String` por
/// `&str` para que las funciones generadas (select_by_id, select_by_<índice>, etc.) acepten
/// referencias en vez de forzar al llamador a pasar un `String` propio.
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

/// Genera el módulo `entity` anidado en el struct, con la constante `TABLE` y el submódulo
/// `columns` con una constante tipada por columna (usadas en el resto de funciones generadas
/// para referenciar tabla/columnas de forma type-safe en vez de por string).
fn build_entity_module(
    struct_name: &syn::Ident,
    schema_name: &str,
    table_name: &str,
    fields: &[FieldInfo],
) -> TokenStream2 {
    let doc = format!("Constant for name of database schema {}", table_name);
    let schema_name_constant = quote! {
        #[doc = #doc]
        pub const SCHEMA: rusqlite_orm::dao::helpers::types::schema::Schema<super::#struct_name> =
            rusqlite_orm::dao::helpers::types::schema::Schema::<super::#struct_name>::new(#schema_name);
    };

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
            #schema_name_constant
            #table_name_constant
            pub mod columns {
                #(#field_constants)*
            }
        }
    }
}

/// Genera el `impl Entity` requerido por el ORM: la lista de columnas, el mapeo de una fila de
/// resultado a la struct (`map_from_row`) y la extracción de valores para persistir
/// (`get_values`).
fn build_entity_trait_impl(
    struct_name: &syn::Ident,
    fields: &[FieldInfo],
    transients: Vec<Ident>,
) -> TokenStream2 {
    let field_name_list = fields.iter().map(|f| {
        let ident = &f.const_ident;
        quote! { self::entity::columns::#ident }
    });

    // Cada campo se lee por posición (idx), asumiendo que el SELECT devuelve las columnas
    // en el mismo orden que FIELDS.
    let map_from_rows_lines = fields.iter().enumerate().map(|(idx, f)| {
        let ident = &f.ident;
        let ty = &f.ty;
        quote! { #ident: row.get::<_, #ty>(#idx)? }
    });

    // Si hay campos transient/relationship, no vienen en la fila: se completan con su valor
    // por defecto vía `..Default::default()`.
    let default_spread = transients.iter().map(|i| 
        quote!{
            , #i: Default::default()
        }
    );

    let get_values_lines = fields.iter().map(|f| {
        let ident = &f.ident;
        quote! { self.#ident.clone().into() }
    });
    let repository = format_ident!("{}Repository", struct_name);

    quote! {
        impl rusqlite_orm::dao::Entity for #struct_name {
            #[doc = "Database schema constant"]
            const SCHEMA: &'static rusqlite_orm::dao::helpers::types::schema::Schema<Self> = &self::entity::SCHEMA;

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
                    #(#default_spread)*
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

/// Genera, para cada `#[relationship(...)]`, un par de métodos `fetch_<campo>_relationship[_in_tx]`
/// que cargan la entidad/lista relacionada en el propio campo, construyendo el `Where` de join
/// a partir de los pares (campo local, columna remota) declarados en el atributo.
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
            // Relación Option<T> => se espera como mucho una fila (fetch_one); Vec<T> => varias.
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

/// Genera, en el `impl` de la entidad, los métodos de instancia `update_by_id`/`delete_by_id`
/// (y sus variantes `_in_tx`) que actualizan/eliminan la fila identificada por sus campos
/// `#[primary_key]`. No se genera nada si la entidad no tiene clave primaria.
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

/// Genera, en el `impl` del repositorio, las funciones asociadas `exists`/`select_by_id`
/// (y sus variantes `_in_tx`) que reciben los campos de la clave primaria como parámetros.
/// No se genera nada si la entidad no tiene clave primaria.
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

/// Genera, en el `impl` del repositorio, cuatro funciones asociadas por cada índice declarado:
/// `select_by_<cols>[_in_tx]` y `count_by_<cols>[_in_tx]`. El nombre de la función se deriva de
/// las columnas variables del índice y, si hay condiciones fijas, de un sufijo `_where_<cond>`.
/// Los índices `unique` devuelven `Option<T>` y no aceptan `order_by`; los no únicos devuelven
/// `Vec<T>` y sí lo aceptan.
fn build_indexes_impl(struct_name: &syn::Ident, indexes: &[IndexDefinition]) -> TokenStream2 {
    if indexes.is_empty() {
        return quote! {};
    }

    let indexes_expand = indexes.iter().map(|index| {
        let fn_name = format_ident!(
            "select_by_{}",
            index.name,
        );
        let fn_name_tx = format_ident!(
            "{}_in_tx",
            fn_name,
        );
        let fn_name_count = format_ident!(
            "{}_by_{}",
            if index.unique {"exists"} else {"count"},
            index.name,
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

        let idx_col_names_idents = index.columns.iter().map(|i| i.ident.clone()).collect::<Vec<syn::Ident>>();
        let doc1=format!("Fetch row{} by {} index", if index.unique {""} else {"s"}, index.name);
        let doc2=format!("{} in transaction", doc1);
        let doc3=format!("{} by {} index", if index.unique {"Exists"} else {"Count rows"}, index.name);
        let doc4=format!("{} in transaction", doc3);

        // Un índice único devuelve como mucho una fila, así que ordenar no tiene sentido:
        // solo los índices no únicos exponen el parámetro `order_by`.
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
                    .where_(#condition).count_in_tx(tx)?>0)
            }
        } else {
            quote! {
                <#repo_ident as rusqlite_orm::dao::Repository<#struct_name>>::select()
                    .where_(#condition).count_in_tx(tx)
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
            pub fn #fn_name_count(#(#select_params),*) -> rusqlite_orm::database::errors::Result<#cnt_ret_type> {
                let mut db = rusqlite_orm::database::DATABASE_INST.lock().unwrap();
                db.run_in_tx(|tx| Self::#fn_name_count_tx(tx, #(#idx_col_names_idents),*))
            }

            #[doc = #doc4]
            pub fn #fn_name_count_tx(tx: &rusqlite_orm::rusqlite::Transaction, #(#select_params),*) -> rusqlite_orm::database::errors::Result<#cnt_ret_type> {
                #cnt_impl
            }
        }
    });

    quote! {
        #(#indexes_expand)*
    }
}

/// Cuando `#[entity(comparable = true)]`, genera `PartialEq`/`Eq` comparando únicamente los
/// campos `#[primary_key]` (dos instancias son iguales si representan la misma fila).
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

/// Cuando `#[entity(hashable = true)]`, genera `std::hash::Hash` hasheando únicamente los
/// campos `#[primary_key]`, coherente con el `Eq` generado por `build_comparable_impl`.
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



/// Metadatos de un campo "persistente" de la entidad (no transient, no relationship).
struct FieldInfo {
    /// Nombre del campo en la struct de Rust.
    ident: syn::Ident,
    /// Nombre de la columna en la base de datos (por defecto el nombre en minúsculas).
    column: String,
    /// Tipo Rust del campo.
    ty: syn::Type,
    /// Identificador de la constante generada para la columna (nombre del campo en mayúsculas).
    const_ident: syn::Ident,
    /// Indica si el campo está marcado con `#[primary_key]`.
    is_id: bool,
}

/// Valores extraídos del atributo `#[entity(...)]` a nivel de struct.
struct EntityAttrs {
    /// Nombre del schema
    schema_name: String,
    /// Nombre de la tabla; si no se especifica se usa el nombre de la struct en minúsculas.
    table_name: String,
    /// Si es `true`, se genera `impl PartialEq`/`Eq` basado en los campos id.
    comparable: bool,
    /// Si es `true`, se genera `impl Hash` basado en los campos id.
    hashable: bool,
}

/// Resultado de analizar todos los campos de la struct anotada.
struct ParsedFields {
    fields: Vec<FieldInfo>,
    transients: Vec<Ident>,
    relationships: Vec<RelationshipDefinition>,
    /// `true` si existe al menos un campo `#[primary_key]`.
    has_id: bool,
}

/// Definición de una relación declarada con `#[relationship(...)]` (campo `Option<T>` o `Vec<T>`).
struct RelationshipDefinition {
    /// Campo de la struct que almacena la relación.
    field: syn::Ident,
    /// `true` si el campo es `Option<T>` (relación a lo sumo 1) o `false` si es `Vec<T>` (a muchos).
    by_id: bool,
    /// Tipo `T` de la entidad relacionada.
    ty: syn::Type,
    /// Pares (campo local, columna remota) usados para construir la condición de join.
    joins: Vec<(Ident, Path)>,
}

/// Definición de un índice declarado con `#[indexes(...)]` o `#[uniques(...)]`.
struct IndexDefinition<'a> {
    /// Nombre del indice
    pub name: String,
    /// Columnas que forman parte del índice y que se reciben como parámetro en las funciones generadas.
    pub columns: Vec<&'a FieldInfo>,
    /// Columnas fijadas a un valor literal constante (p. ej. `#[indexes((status = "active"))]`).
    pub conditions: Option<Vec<(&'a FieldInfo, syn::Lit)>>,
    /// Si es `true` el índice es único y las funciones generadas devuelven `Option` en vez de `Vec`.
    pub unique: bool,
}