mod ty;
mod ty_boxed;
mod ty_delegate;
mod ty_enum;
mod ty_general_list;
mod ty_optional;
mod ty_primitive;
mod ty_primitive_list;
mod ty_struct;

pub use ty::*;
pub use ty_boxed::*;
pub use ty_delegate::*;
pub use ty_enum::*;
pub use ty_general_list::*;
pub use ty_optional::*;
pub use ty_primitive::*;
pub use ty_primitive_list::*;
pub use ty_struct::*;

use std::borrow::Cow;
use std::collections::HashSet;

use crate::ir::IrType::*;
use crate::ir::*;
use crate::others::*;

pub const HANDLER_NAME: &str = "FLUTTER_RUST_BRIDGE_HANDLER";

pub struct Output {
    pub code: String,
    pub extern_func_names: Vec<String>,
}

pub fn generate(api_file: &IrFile, rust_wire_mod: &str) -> Output {
    let mut generator = Generator::new();
    let code = generator.generate(api_file, rust_wire_mod);

    Output {
        code,
        extern_func_names: generator.extern_func_collector.names,
    }
}

struct Generator {
    extern_func_collector: ExternFuncCollector,
}

impl Generator {
    fn new() -> Self {
        Self {
            extern_func_collector: ExternFuncCollector::new(),
        }
    }

    fn generate(&mut self, api_file: &IrFile, rust_wire_mod: &str) -> String {
        let mut lines: Vec<String> = vec![];

        let distinct_input_types = api_file.distinct_types(true, false);
        let distinct_output_types = api_file.distinct_types(false, true);

        lines.push(r#"#![allow(non_camel_case_types, unused, clippy::redundant_closure, clippy::useless_conversion, clippy::unit_arg, non_snake_case)]"#.to_string());
        lines.push(CODE_HEADER.to_string());

        lines.push(String::new());
        lines.push(format!("use crate::{}::*;", rust_wire_mod));
        lines.push("use flutter_rust_bridge::*;".to_string());
        lines.push(String::new());

        lines.push(self.section_header_comment("imports"));
        lines.extend(self.generate_imports(
            api_file,
            rust_wire_mod,
            &distinct_input_types,
            &distinct_output_types,
        ));
        lines.push(String::new());

        lines.push(self.section_header_comment("wire functions"));
        lines.extend(api_file.funcs.iter().map(|f| self.generate_wire_func(f)));

        lines.push(self.section_header_comment("wire structs"));
        lines.extend(
            distinct_input_types
                .iter()
                .map(|ty| self.generate_wire_struct(ty, api_file)),
        );

        lines.push(self.section_header_comment("wire enums"));
        lines.extend(distinct_input_types.iter().filter_map(|ty| match ty {
            IrType::EnumRef(enu) => Some(self.generate_wire_enum(enu, api_file)),
            _ => None,
        }));

        lines.push(self.section_header_comment("allocate functions"));
        lines.extend(
            distinct_input_types
                .iter()
                .map(|f| self.generate_allocate_funcs(f)),
        );

        lines.push(self.section_header_comment("impl Wire2Api"));
        lines.push(self.generate_wire2api_misc().to_string());
        lines.extend(
            distinct_input_types
                .iter()
                .map(|ty| self.generate_wire2api_func(ty, api_file)),
        );

        lines.push(self.section_header_comment("impl NewWithNullPtr"));
        lines.push(self.generate_new_with_nullptr_misc().to_string());
        lines.extend(
            distinct_input_types
                .iter()
                .map(|ty| self.generate_new_with_nullptr_func(ty, api_file)),
        );

        lines.push(self.section_header_comment("impl IntoDart"));
        lines.extend(
            distinct_output_types
                .iter()
                .map(|ty| self.generate_impl_intodart(ty, api_file)),
        );

        lines.push(self.section_header_comment("executor"));
        lines.push(self.generate_executor(api_file));

        lines.push(self.section_header_comment("sync execution mode utility"));
        lines.push(self.generate_sync_execution_mode_utility());

        lines.join("\n")
    }

    fn section_header_comment(&self, section_name: &str) -> String {
        format!("// Section: {}\n", section_name)
    }

    fn generate_imports(
        &self,
        api_file: &IrFile,
        rust_wire_mod: &str,
        distinct_input_types: &[IrType],
        distinct_output_types: &[IrType],
    ) -> impl Iterator<Item = String> {
        let input_type_imports = distinct_input_types
            .iter()
            .map(|api_type| self.generate_import(api_type, api_file));
        let output_type_imports = distinct_output_types
            .iter()
            .map(|api_type| self.generate_import(api_type, api_file));

        input_type_imports
            .chain(output_type_imports)
            // Filter out `None` and unwrap
            .flatten()
            // Don't include imports from the API file
            .filter(|import| !import.starts_with(&format!("use crate::{}::", rust_wire_mod)))
            // de-duplicate
            .collect::<HashSet<String>>()
            .into_iter()
    }

    fn generate_import(&self, api_type: &IrType, api_file: &IrFile) -> Option<String> {
        match api_type {
            EnumRef(enum_ref) => {
                let api_enum = enum_ref.get(api_file);
                Some(format!("use {};", api_enum.path.join("::")))
            }
            StructRef(struct_ref) => {
                let api_struct = struct_ref.get(api_file);
                if api_struct.path.is_some() {
                    Some(format!(
                        "use {};",
                        api_struct.path.as_ref().unwrap().join("::")
                    ))
                } else {
                    None
                }
            }
            Optional(optional_ref) => self.generate_import(&optional_ref.inner, api_file),
            GeneralList(general_list_ref) => {
                self.generate_import(&general_list_ref.inner, api_file)
            }
            Boxed(boxed_ref) => self.generate_import(&boxed_ref.inner, api_file),
            _ => None,
        }
    }

    fn generate_executor(&mut self, api_file: &IrFile) -> String {
        if api_file.has_executor {
            "/* nothing since executor detected */".to_string()
        } else {
            format!(
                "support::lazy_static! {{
                pub static ref {}: support::DefaultHandler = Default::default();
            }}
            ",
                HANDLER_NAME
            )
        }
    }

    fn generate_sync_execution_mode_utility(&mut self) -> String {
        self.extern_func_collector.generate(
            "free_WireSyncReturnStruct",
            &["val: support::WireSyncReturnStruct"],
            None,
            "unsafe { let _ = support::vec_from_leak_ptr(val.ptr, val.len); }",
        )
    }

    fn generate_wire_func(&mut self, func: &IrFunc) -> String {
        let params = [
            if func.mode.has_port_argument() {
                vec!["port_: i64".to_string()]
            } else {
                vec![]
            },
            func.inputs
                .iter()
                .map(|field| {
                    format!(
                        "{}: {}{}",
                        field.name.rust_style(),
                        field.ty.rust_wire_modifier(),
                        field.ty.rust_wire_type()
                    )
                })
                .collect::<Vec<_>>(),
        ]
        .concat();

        let inner_func_params = [
            match func.mode {
                ApiFuncMode::Normal | ApiFuncMode::Sync => vec![],
                ApiFuncMode::Stream => vec!["task_callback.stream_sink()".to_string()],
            },
            func.inputs
                .iter()
                .map(|field| format!("api_{}", field.name.rust_style()))
                .collect::<Vec<_>>(),
        ]
        .concat();

        let wrap_info_obj = format!(
            "WrapInfo{{ debug_name: \"{}\", port: {}, mode: FfiCallMode::{} }}",
            func.name,
            if func.mode.has_port_argument() {
                "Some(port_)"
            } else {
                "None"
            },
            func.mode.ffi_call_mode(),
        );

        let code_wire2api = func
            .inputs
            .iter()
            .map(|field| {
                format!(
                    "let api_{} = {}.wire2api();",
                    field.name.rust_style(),
                    field.name.rust_style()
                )
            })
            .collect::<Vec<_>>()
            .join("");

        let code_call_inner_func = format!("{}({})", func.name, inner_func_params.join(", "));

        let code_call_inner_func_result = if func.fallible {
            code_call_inner_func
        } else {
            format!("Ok({})", code_call_inner_func)
        };

        let (handler_func_name, return_type, code_closure) = match func.mode {
            ApiFuncMode::Sync => (
                "wrap_sync",
                Some("support::WireSyncReturnStruct"),
                format!(
                    "{}
                    {}",
                    code_wire2api, code_call_inner_func_result,
                ),
            ),
            ApiFuncMode::Normal | ApiFuncMode::Stream => (
                "wrap",
                None,
                format!(
                    "{}
                    move |task_callback| {}
                    ",
                    code_wire2api, code_call_inner_func_result,
                ),
            ),
        };

        self.extern_func_collector.generate(
            &func.wire_func_name(),
            &params
                .iter()
                .map(std::ops::Deref::deref)
                .collect::<Vec<_>>(),
            return_type,
            &format!(
                "
                {}.{}({}, move || {{
                    {}
                }})
                ",
                HANDLER_NAME, handler_func_name, wrap_info_obj, code_closure,
            ),
        )
    }

    fn generate_wire_struct(&mut self, ty: &IrType, api_file: &IrFile) -> String {
        // println!("generate_wire_struct: {:?}", ty);
        let fields = match ty {
            PrimitiveList(list) => vec![
                format!("ptr: *mut {}", list.primitive.rust_wire_type()),
                "len: i32".to_string(),
            ],
            Delegate(ty @ IrTypeDelegate::StringList) => vec![
                format!("ptr: *mut *mut {}", ty.get_delegate().rust_wire_type()),
                "len: i32".to_owned(),
            ],
            GeneralList(list) => vec![
                format!(
                    "ptr: *mut {}{}",
                    list.inner.rust_ptr_modifier(),
                    list.inner.rust_wire_type()
                ),
                "len: i32".to_string(),
            ],
            StructRef(s) => {
                let s = s.get(api_file);
                s.fields
                    .iter()
                    .map(|field| {
                        format!(
                            "{}: {}{}",
                            field.name.rust_style(),
                            field.ty.rust_wire_modifier(),
                            field.ty.rust_wire_type()
                        )
                    })
                    .collect()
            }
            Primitive(_) | Delegate(_) | Boxed(_) | Optional(_) | EnumRef(_) => {
                return "".to_string()
            }
        };

        format!(
            r###"
        #[repr(C)]
        #[derive(Clone)]
        pub struct {} {{
            {}
        }}
        "###,
            ty.rust_wire_type(),
            fields.join(",\n"),
        )
    }

    fn generate_wire_enum(&mut self, enu: &IrTypeEnumRef, file: &IrFile) -> String {
        let src = enu.get(file);
        if !src.is_struct() {
            return "".to_owned();
        }
        let variant_structs = src
            .variants()
            .iter()
            .map(|variant| {
                let fields = match &variant.kind {
                    ApiVariantKind::Value => vec![],
                    ApiVariantKind::Struct(s) => s
                        .fields
                        .iter()
                        .map(|field| {
                            format!(
                                "{}: {}{},",
                                field.name.rust_style(),
                                field.ty.rust_wire_modifier(),
                                field.ty.rust_wire_type()
                            )
                        })
                        .collect(),
                };
                format!(
                    "#[repr(C)]
                    #[derive(Clone)]
                    pub struct {}_{} {{ {} }}",
                    enu.name,
                    variant.name,
                    fields.join("\n")
                )
            })
            .collect::<Vec<_>>();
        let union_fields = src
            .variants()
            .iter()
            .map(|variant| format!("{0}: *mut {1}_{0},", variant.name, enu.name))
            .collect::<Vec<_>>();
        format!(
            "#[repr(C)]
            #[derive(Clone)]
            pub struct {0} {{ tag: i32, kind: *mut {1}Kind }}

            #[repr(C)]
            pub union {1}Kind {{
                {2}
            }}

            {3}",
            enu.rust_wire_type(),
            enu.name,
            union_fields.join("\n"),
            variant_structs.join("\n\n")
        )
    }

    fn generate_list_allocate_func(
        &mut self,
        safe_ident: &str,
        list: &impl ApiTypeChild,
        inner: &IrType,
    ) -> String {
        self.extern_func_collector.generate(
            &format!("new_{}", safe_ident),
            &["len: i32"],
            Some(&[
                list.rust_wire_modifier().as_str(),
                list.rust_wire_type().as_str()
            ].concat()),
            &format!(
                "let wrap = {} {{ ptr: support::new_leak_vec_ptr(<{}{}>::new_with_null_ptr(), len), len }};
                support::new_leak_box_ptr(wrap)",
                list.rust_wire_type(),
                inner.rust_ptr_modifier(),
                inner.rust_wire_type()
            ),
        )
    }

    fn generate_allocate_funcs(&mut self, ty: &IrType) -> String {
        // println!("generate_allocate_funcs: {:?}", ty);

        match ty {
            PrimitiveList(list) => self.extern_func_collector.generate(
                &format!("new_{}", list.safe_ident()),
                &["len: i32"],
                Some(&format!("{}{}", list.rust_wire_modifier(), list.rust_wire_type())),
                &format!(
                    "let ans = {} {{ ptr: support::new_leak_vec_ptr(Default::default(), len), len }};
                support::new_leak_box_ptr(ans)",
                    list.rust_wire_type(),
                ),
            ),
            GeneralList(list) =>
                self.generate_list_allocate_func(&ty.safe_ident(), list, &list.inner),
            Delegate(list @ IrTypeDelegate::StringList) =>
                self.generate_list_allocate_func(&ty.safe_ident(), list, &list.get_delegate()),
            Boxed(b) => {
                match &*b.inner {
                    Primitive(prim) => {
                        self.extern_func_collector.generate(
                            &format!("new_{}", ty.safe_ident()),
                            &[&format!("value: {}", prim.rust_wire_type())],
                            Some(&format!("*mut {}", prim.rust_wire_type())),
                            "support::new_leak_box_ptr(value)",
                        )
                    }
                    inner => {
                        self.extern_func_collector.generate(
                            &format!("new_{}", ty.safe_ident()),
                            &[],
                            Some(&[ty.rust_wire_modifier(), ty.rust_wire_type()].concat()),
                            &format!(
                                "support::new_leak_box_ptr({}::new_with_null_ptr())",
                                inner.rust_wire_type()
                            ),
                        )
                    }
                }
            },
            Primitive(_) | Delegate(_) | StructRef(_) | EnumRef(_) | Optional(_) => String::new(),
        }
    }

    fn generate_wire2api_misc(&self) -> &'static str {
        r"pub trait Wire2Api<T> {
            fn wire2api(self) -> T;
        }
        
        impl<T, S> Wire2Api<Option<T>> for *mut S
            where
                *mut S: Wire2Api<T>
        {
            fn wire2api(self) -> Option<T> {
                if self.is_null() {
                    None
                } else {
                    Some(self.wire2api())
                }
            }
        }
        "
    }

    fn generate_wire2api_func(&mut self, ty: &IrType, api_file: &IrFile) -> String {
        // println!("generate_wire2api_func: {:?}", ty);
        let body: Cow<str> = match ty {
            Primitive(_) => "self".into(),
            Delegate(IrTypeDelegate::String) => "let vec: Vec<u8> = self.wire2api();
            String::from_utf8_lossy(&vec).into_owned()"
                .into(),
            Delegate(IrTypeDelegate::SyncReturnVecU8) => "/*unsupported*/".into(),
            Delegate(IrTypeDelegate::ZeroCopyBufferVecPrimitive(_)) => {
                "ZeroCopyBuffer(self.wire2api())".into()
            }
            PrimitiveList(_) => "unsafe {
                let wrap = support::box_from_leak_ptr(self);
                support::vec_from_leak_ptr(wrap.ptr, wrap.len)
            }"
                .into(),
            GeneralList(_) | Delegate(IrTypeDelegate::StringList) => "
            let vec = unsafe {
                let wrap = support::box_from_leak_ptr(self);
                support::vec_from_leak_ptr(wrap.ptr, wrap.len)
            };
            vec.into_iter().map(Wire2Api::wire2api).collect()"
                .into(),
            Boxed(IrTypeBoxed { inner: box_inner, exist_in_real_api }) => match (box_inner.as_ref(), exist_in_real_api) {
                (IrType::Primitive(_), false) => "unsafe { *support::box_from_leak_ptr(self) }".into(),
                (IrType::Primitive(_), true) => "unsafe { support::box_from_leak_ptr(self) }".into(),
                _ => "let wrap = unsafe { support::box_from_leak_ptr(self) }; (*wrap).wire2api().into()".into()
            }
            StructRef(struct_ref) => {
                let api_struct = struct_ref.get(api_file);
                let fields_str = &api_struct
                    .fields
                    .iter()
                    .map(|field| {
                        format!(
                            "{} self.{}.wire2api()",
                            if api_struct.is_fields_named {
                                field.name.rust_style().to_string() + ": "
                            } else {
                                String::new()
                            },
                            field.name.rust_style()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");

                let (left, right) = api_struct.brackets_pair();
                format!("{}{}{}{}", ty.rust_api_type(), left, fields_str, right).into()
            }
            EnumRef(enu) if enu.is_struct => {
                let enu = enu.get(api_file);
                let variants = enu.variants().iter().enumerate()
                    .map(|(idx, variant)| {
                        match &variant.kind {
                            ApiVariantKind::Value => {
                                format!("{} => {}::{},", idx, enu.name, variant.name)
                            },
                            ApiVariantKind::Struct(st) => {
                                let fields: Vec<_> = st.fields
                                    .iter()
                                    .map(|field| {
                                        if st.is_fields_named {
                                            format!("{0}: ans.{0}.wire2api()", field.name.rust_style())
                                        } else {
                                            format!("ans.{}.wire2api()", field.name.rust_style())
                                        }
                                    }).collect();
                                let (left, right) = st.brackets_pair();
                                format!(
                                    "{} => unsafe {{
                                        let ans = support::box_from_leak_ptr(self.kind);
                                        let ans = support::box_from_leak_ptr(ans.{2});
                                        {}::{2}{3}{4}{5}
                                    }}",
                                    idx, enu.name, variant.name, left, fields.join(","), right
                                )
                            }
                        }
                    }).collect::<Vec<_>>();
                format!(
                    "match self.tag {{
                        {}
                        _ => unreachable!(),
                    }}",
                    variants.join("\n"),
                ).into()
            }
            EnumRef(enu) => {
                let enu = enu.get(api_file);
                let variants = enu
                    .variants()
                    .iter()
                    .enumerate()
                    .map(|(idx, variant)| format!("{} => {}::{},", idx, enu.name, variant.name))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "match self {{
                        {}
                        _ => unreachable!(\"Invalid variant for {}: {{}}\", self),
                    }}",
                    variants, enu.name
                )
                    .into()
            }
            // handled by common impl
            Optional(_) => return String::new(),
        };

        format!(
            "impl Wire2Api<{}> for {} {{
            fn wire2api(self) -> {} {{
                {}
            }}
        }}
        ",
            ty.rust_api_type(),
            ty.rust_wire_modifier() + &ty.rust_wire_type(),
            ty.rust_api_type(),
            body,
        )
    }

    fn generate_new_with_nullptr_misc(&self) -> &'static str {
        "pub trait NewWithNullPtr {
            fn new_with_null_ptr() -> Self;
        }
        
        impl<T> NewWithNullPtr for *mut T {
            fn new_with_null_ptr() -> Self {
                std::ptr::null_mut()
            }
        }
        "
    }

    fn generate_new_with_nullptr_func(&mut self, ty: &IrType, api_file: &IrFile) -> String {
        match ty {
            StructRef(st) => self
                .generate_new_with_nullptr_func_for_struct(st.get(api_file), &ty.rust_wire_type()),
            EnumRef(e) if e.is_struct => {
                self.generate_new_with_nullptr_func_for_enum(e.get(api_file), &ty.rust_wire_type())
            }
            Primitive(_) | Delegate(_) | PrimitiveList(_) | GeneralList(_) | Boxed(_)
            | Optional(_) | EnumRef(_) => String::new(),
        }
    }

    fn generate_impl_intodart(&mut self, ty: &IrType, api_file: &IrFile) -> String {
        // println!("generate_impl_intodart: {:?}", ty);
        match ty {
            StructRef(s) => self.generate_impl_intodart_for_struct(s.get(api_file)),
            EnumRef(e) if e.is_struct => {
                self.generate_impl_intodart_for_enum_struct(e.get(api_file))
            }
            EnumRef(enu) => self.generate_impl_intodart_for_enum(enu.get(api_file)),
            Primitive(_) | Delegate(_) | PrimitiveList(_) | GeneralList(_) | Boxed(_)
            | Optional(_) => "".to_string(),
        }
    }

    fn generate_new_with_nullptr_func_for_enum(
        &mut self,
        enu: &ApiEnum,
        rust_wire_type: &str,
    ) -> String {
        fn init_of(ty: &IrType) -> &str {
            if ty.rust_wire_is_pointer() {
                "core::ptr::null_mut()"
            } else {
                "Default::default()"
            }
        }
        let inflators = enu
            .variants()
            .iter()
            .filter_map(|variant| {
                let typ = format!("{}_{}", enu.name, variant.name);
                let body: Vec<_> = if let ApiVariantKind::Struct(st) = &variant.kind {
                    st.fields
                        .iter()
                        .map(|field| format!("{}: {}", field.name.rust_style(), init_of(&field.ty)))
                        .collect()
                } else {
                    return None;
                };
                Some(self.extern_func_collector.generate(
                    &format!("inflate_{}", typ),
                    &[],
                    Some(&format!("*mut {}Kind", enu.name)),
                    &format!(
                        "support::new_leak_box_ptr({}Kind {{
                        {}: support::new_leak_box_ptr({} {{
                            {}
                        }})
                    }})",
                        enu.name,
                        variant.name.rust_style(),
                        typ,
                        body.join(",")
                    ),
                ))
            })
            .collect::<Vec<_>>();
        format!(
            "impl NewWithNullPtr for {} {{
                fn new_with_null_ptr() -> Self {{
                    Self {{
                        tag: -1,
                        kind: core::ptr::null_mut(),
                    }}
                }}
            }}
            {}",
            rust_wire_type,
            inflators.join("\n\n")
        )
    }

    fn generate_new_with_nullptr_func_for_struct(
        &self,
        s: &ApiStruct,
        rust_wire_type: &str,
    ) -> String {
        let body = {
            s.fields
                .iter()
                .map(|field| {
                    format!(
                        "{}: {},",
                        field.name.rust_style(),
                        if field.ty.rust_wire_is_pointer() {
                            "core::ptr::null_mut()"
                        } else {
                            "Default::default()"
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!(
            r#"impl NewWithNullPtr for {} {{
                    fn new_with_null_ptr() -> Self {{
                        Self {{ {} }}
                    }}
                }}
            "#,
            rust_wire_type, body,
        )
    }

    fn generate_impl_intodart_for_struct(&mut self, s: &ApiStruct) -> String {
        // println!("generate_impl_intodart_for_struct: {}", s.name);
        let body = s
            .fields
            .iter()
            .map(|field| {
                format!(
                    "self.{}.into_dart()",
                    field.name_rust_style(s.is_fields_named)
                )
            })
            .collect::<Vec<_>>()
            .join(",\n");

        format!(
            "impl support::IntoDart for {} {{
                fn into_dart(self) -> support::DartCObject {{
                    vec![
                        {}
                    ].into_dart()
                }}
            }}
            impl support::IntoDartExceptPrimitive for {} {{}}
            ",
            s.name, body, s.name,
        )
    }

    fn generate_impl_intodart_for_enum(&mut self, enu: &ApiEnum) -> String {
        let variants = enu
            .variants()
            .iter()
            .enumerate()
            .map(|(idx, variant)| format!("Self::{} => {},", variant.name, idx))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "impl support::IntoDart for {} {{
                fn into_dart(self) -> support::DartCObject {{
                    match self {{
                        {}
                    }}.into_dart()
                }}
            }}
            ",
            enu.name, variants
        )
    }

    fn generate_impl_intodart_for_enum_struct(&mut self, enu: &ApiEnum) -> String {
        let variants =
            enu.variants()
                .iter()
                .enumerate()
                .map(|(idx, variant)| {
                    let tag = format!("{}.into_dart()", idx);
                    match &variant.kind {
                        ApiVariantKind::Value => {
                            format!("Self::{} => vec![{}],", variant.name, tag)
                        }
                        ApiVariantKind::Struct(s) => {
                            let fields = Some(tag)
                                .into_iter()
                                .chain(s.fields.iter().map(|field| {
                                    format!("{}.into_dart()", field.name.rust_style())
                                }))
                                .collect::<Vec<_>>();
                            let pattern = s
                                .fields
                                .iter()
                                .map(|field| field.name.rust_style().to_owned())
                                .collect::<Vec<_>>();
                            let (left, right) = s.brackets_pair();
                            format!(
                                "Self::{}{}{}{} => vec![{}],",
                                variant.name,
                                left,
                                pattern.join(","),
                                right,
                                fields.join(",")
                            )
                        }
                    }
                })
                .collect::<Vec<_>>();
        format!(
            "impl support::IntoDart for {} {{
                fn into_dart(self) -> support::DartCObject {{
                    match self {{
                        {}
                    }}.into_dart()
                }}
            }}
            ",
            enu.name,
            variants.join("\n")
        )
    }
}

struct ExternFuncCollector {
    names: Vec<String>,
}

impl ExternFuncCollector {
    fn new() -> Self {
        ExternFuncCollector { names: vec![] }
    }

    fn generate(
        &mut self,
        func_name: &str,
        params: &[&str],
        return_type: Option<&str>,
        body: &str,
    ) -> String {
        self.names.push(func_name.to_string());

        format!(
            r#"
                #[no_mangle]
                pub extern "C" fn {}({}) {} {{
                    {}
                }}
            "#,
            func_name,
            params.join(", "),
            return_type.map_or("".to_string(), |r| format!("-> {}", r)),
            body,
        )
    }
}