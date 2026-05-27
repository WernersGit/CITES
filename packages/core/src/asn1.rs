pub mod cam {
    pub mod v1 {
        #![allow(non_upper_case_globals)]
        #![allow(non_camel_case_types)]
        #![allow(non_snake_case)]
        #![allow(dead_code)]
        include!(concat!(env!("OUT_DIR"), "/cam_v1_bindings.rs"));
    }

    pub mod v2 {
        #![allow(non_upper_case_globals)]
        #![allow(non_camel_case_types)]
        #![allow(non_snake_case)]
        #![allow(dead_code)]
        include!(concat!(env!("OUT_DIR"), "/cam_v2_bindings.rs"));
    }
}

pub mod denm {
    pub mod v1 {
        #![allow(non_upper_case_globals)]
        #![allow(non_camel_case_types)]
        #![allow(non_snake_case)]
        #![allow(dead_code)]
        include!(concat!(env!("OUT_DIR"), "/denm_v1_bindings.rs"));
    }

    pub mod v2 {
        #![allow(non_upper_case_globals)]
        #![allow(non_camel_case_types)]
        #![allow(non_snake_case)]
        #![allow(dead_code)]
        include!(concat!(env!("OUT_DIR"), "/denm_v2_bindings.rs"));
    }
}

pub mod cpm {
    pub mod v2 {
        #![allow(non_upper_case_globals)]
        #![allow(non_camel_case_types)]
        #![allow(non_snake_case)]
        #![allow(dead_code)]
        include!(concat!(env!("OUT_DIR"), "/cpm_v2_bindings.rs"));
    }
}

pub mod is {
    pub mod v2 {
        #![allow(non_upper_case_globals)]
        #![allow(non_camel_case_types)]
        #![allow(non_snake_case)]
        #![allow(dead_code)]
        include!(concat!(env!("OUT_DIR"), "/is_v2_bindings.rs"));
    }
}
