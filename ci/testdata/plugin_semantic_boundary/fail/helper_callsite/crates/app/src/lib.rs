use uptrakit_shared_types::PluginTypeId;
use uptrakit_shared_types::PluginTypeId as Id;

fn make_id(id: &PluginTypeId) -> &PluginTypeId {
    id
}

pub fn should_be_scanned(id: &PluginTypeId) {
    let _ = id.is_package_manager();
    let _ = PluginTypeId::display_name(id);
    let _ = PluginTypeId :: display_name(id);
    let _ = make_id(id).display_name();
    let _ = { make_id(id) }.display_name();
}

pub struct Holder {
    pub plugin_types: Vec<PluginTypeId>,
}

impl Holder {
    pub fn indexed_receiver_should_be_rejected(&self) {
        let _ = self.plugin_types[0].display_name();
    }
}

pub fn alias_typed_local_should_be_rejected() {
    type Id = PluginTypeId;
    let alias_id: Id = PluginTypeId::from_static("alias_type_fixture");
    let _ = alias_id.display_name();
}

pub fn alias_associated_display_name_should_be_rejected(id: &PluginTypeId) {
    type Id = PluginTypeId;
    let _ = Id::display_name(id);
}

pub fn import_alias_associated_is_package_manager_should_be_rejected(id: &Id) {
    let _ = Id::is_package_manager(id);
}

pub fn collected_index_receiver_should_be_rejected(ids: Vec<PluginTypeId>) {
    let _ = ids.into_iter().collect::<Vec<_>>()[0].display_name();
}

fn make_plugin_type() -> PluginTypeId {
    PluginTypeId::from_static("direct_factory_fixture")
}

pub fn function_return_receiver_should_be_rejected() {
    make_plugin_type().display_name();
}

pub fn multiline_method_receiver_should_be_rejected(id: &PluginTypeId) {
    let _ = id
        .display_name();
}

pub fn multiline_complex_receiver_should_be_rejected(id: &PluginTypeId) {
    let _ = make_id(id)
        .display_name();
}

pub fn multiline_associated_display_name_should_be_rejected(id: &PluginTypeId) {
    let _ = PluginTypeId
        ::display_name(id);
}

pub fn multiline_associated_is_package_manager_should_be_rejected(id: &Id) {
    let _ = Id
        ::is_package_manager(id);
}

fn source() -> u8 {
    7
}

fn make_plugin_type_with_generic<T>(value: T) -> PluginTypeId {
    let _ = value;
    PluginTypeId::from_static("generic_factory_fixture")
}

pub fn function_return_receiver_with_turbofish_should_be_rejected() {
    make_plugin_type_with_generic::<u8>(source()).display_name();
}

pub struct DirectFieldHolder {
    pub plugin_type: PluginTypeId,
}

impl DirectFieldHolder {
    pub fn direct_field_receiver_should_be_rejected(&self) {
        let _ = self.plugin_type.display_name();
        let _ = self.plugin_type.is_package_manager();
    }

    pub fn multiline_direct_field_receiver_should_be_rejected(&self) {
        let _ = self.plugin_type
            .display_name();
        let _ = self.plugin_type
            .is_package_manager();
    }
}
