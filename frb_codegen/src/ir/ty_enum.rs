use crate::ir::IrType::{EnumRef, StructRef};
use crate::ir::*;
use convert_case::{Case, Casing};

#[derive(Debug, Clone)]
pub struct IrTypeEnumRef {
    pub name: String,
    pub is_struct: bool,
}

impl IrTypeEnumRef {
    pub fn get<'a>(&self, file: &'a IrFile) -> &'a ApiEnum {
        &file.enum_pool[&self.name]
    }
}

impl ApiTypeChild for IrTypeEnumRef {
    fn visit_children_types<F: FnMut(&IrType) -> bool>(&self, f: &mut F, api_file: &IrFile) {
        let enu = self.get(api_file);
        for variant in enu.variants() {
            if let ApiVariantKind::Struct(st) = &variant.kind {
                st.fields
                    .iter()
                    .for_each(|field| field.ty.visit_types(f, api_file));
            }
        }
    }

    fn safe_ident(&self) -> String {
        self.dart_api_type().to_case(Case::Snake)
    }
    fn dart_api_type(&self) -> String {
        self.name.to_string()
    }
    fn dart_wire_type(&self) -> String {
        if self.is_struct {
            self.rust_wire_type()
        } else {
            "int".to_owned()
        }
    }
    fn rust_api_type(&self) -> String {
        self.name.to_string()
    }
    fn rust_wire_type(&self) -> String {
        if self.is_struct {
            format!("wire_{}", self.name)
        } else {
            "i32".to_owned()
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApiEnum {
    pub name: String,
    pub path: Vec<String>,
    pub comments: Vec<IrComment>,
    _variants: Vec<ApiVariant>,
    _is_struct: bool,
}

impl ApiEnum {
    pub fn new(
        name: String,
        path: Vec<String>,
        comments: Vec<IrComment>,
        mut variants: Vec<ApiVariant>,
    ) -> Self {
        fn wrap_box(ty: IrType) -> IrType {
            match ty {
                StructRef(_)
                | EnumRef(IrTypeEnumRef {
                    is_struct: true, ..
                }) => IrType::Boxed(IrTypeBoxed {
                    exist_in_real_api: false,
                    inner: Box::new(ty),
                }),
                _ => ty,
            }
        }
        let _is_struct = variants
            .iter()
            .any(|variant| !matches!(variant.kind, ApiVariantKind::Value));
        if _is_struct {
            variants = variants
                .into_iter()
                .map(|variant| ApiVariant {
                    kind: match variant.kind {
                        ApiVariantKind::Struct(st) => ApiVariantKind::Struct(ApiStruct {
                            fields: st
                                .fields
                                .into_iter()
                                .map(|field| IrField {
                                    ty: wrap_box(field.ty),
                                    ..field
                                })
                                .collect(),
                            ..st
                        }),
                        _ => variant.kind,
                    },
                    ..variant
                })
                .collect::<Vec<_>>();
        }
        Self {
            name,
            path,
            comments,
            _variants: variants,
            _is_struct,
        }
    }

    pub fn variants(&self) -> &[ApiVariant] {
        &self._variants
    }

    pub fn is_struct(&self) -> bool {
        self._is_struct
    }
}

#[derive(Debug, Clone)]
pub struct ApiVariant {
    pub name: IrIdent,
    pub comments: Vec<IrComment>,
    pub kind: ApiVariantKind,
}

#[derive(Debug, Clone)]
pub enum ApiVariantKind {
    Value,
    Struct(ApiStruct),
}