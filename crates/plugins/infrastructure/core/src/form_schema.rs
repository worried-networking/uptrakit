// Re-export form schema builder types so plugin crates don't need a direct compat dependency.
pub type FormFieldDescriptor = crate::extension_compat::FieldDef;
pub type FormFieldType = crate::extension_compat::FieldType;
pub type FormSelectOptionDescriptor = crate::extension_compat::SelectOption;
pub type FormSelectSourceDescriptor = crate::extension_compat::SelectSource;

#[doc(hidden)]
pub type FieldDef = FormFieldDescriptor;
#[doc(hidden)]
pub type FieldType = FormFieldType;
#[doc(hidden)]
pub type SelectOption = FormSelectOptionDescriptor;
#[doc(hidden)]
pub type SelectSource = FormSelectSourceDescriptor;
