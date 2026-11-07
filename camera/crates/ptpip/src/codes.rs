//! Codes PTP (ISO 15740) et extensions Nikon.
//!
//! Les codes Nikon viennent de libgphoto2 (`camlibs/ptp2/ptp.h`), seule source
//! publique ; leur prise en charge par une COOLPIX donnée se vérifie avec
//! `GetDeviceInfo` (commande `p900ctl probe`).

// --- Opérations standard -----------------------------------------------------
pub const OC_GET_DEVICE_INFO: u16 = 0x1001;
pub const OC_OPEN_SESSION: u16 = 0x1002;
pub const OC_CLOSE_SESSION: u16 = 0x1003;
pub const OC_GET_STORAGE_IDS: u16 = 0x1004;
pub const OC_GET_STORAGE_INFO: u16 = 0x1005;
pub const OC_GET_OBJECT_HANDLES: u16 = 0x1007;
pub const OC_GET_OBJECT_INFO: u16 = 0x1008;
pub const OC_GET_OBJECT: u16 = 0x1009;
pub const OC_GET_THUMB: u16 = 0x100A;
pub const OC_DELETE_OBJECT: u16 = 0x100B;
pub const OC_INITIATE_CAPTURE: u16 = 0x100E;
pub const OC_GET_DEVICE_PROP_DESC: u16 = 0x1014;
pub const OC_GET_DEVICE_PROP_VALUE: u16 = 0x1015;
pub const OC_SET_DEVICE_PROP_VALUE: u16 = 0x1016;

// --- Opérations Nikon ---------------------------------------------------------
pub const OC_NIKON_CAPTURE_SDRAM: u16 = 0x90C0;
pub const OC_NIKON_AF_DRIVE: u16 = 0x90C1;
pub const OC_NIKON_CHANGE_CAMERA_MODE: u16 = 0x90C2;
pub const OC_NIKON_CHECK_EVENT: u16 = 0x90C7;
pub const OC_NIKON_DEVICE_READY: u16 = 0x90C8;
pub const OC_NIKON_GET_VENDOR_PROP_CODES: u16 = 0x90CA;
pub const OC_NIKON_START_LIVEVIEW: u16 = 0x9201;
pub const OC_NIKON_END_LIVEVIEW: u16 = 0x9202;
pub const OC_NIKON_GET_LIVEVIEW_IMG: u16 = 0x9203;
pub const OC_NIKON_MF_DRIVE: u16 = 0x9204;
pub const OC_NIKON_INITIATE_CAPTURE_REC_IN_MEDIA: u16 = 0x9207;
pub const OC_NIKON_START_MOVIE_REC: u16 = 0x920A;
pub const OC_NIKON_END_MOVIE_REC: u16 = 0x920B;

// --- Réponses -----------------------------------------------------------------
pub const RC_OK: u16 = 0x2001;
pub const RC_GENERAL_ERROR: u16 = 0x2002;
pub const RC_OPERATION_NOT_SUPPORTED: u16 = 0x2005;
pub const RC_INVALID_OBJECT_HANDLE: u16 = 0x2009;
pub const RC_DEVICE_PROP_NOT_SUPPORTED: u16 = 0x200A;
pub const RC_ACCESS_DENIED: u16 = 0x200F;
pub const RC_NO_THUMBNAIL: u16 = 0x2010;
pub const RC_DEVICE_BUSY: u16 = 0x2019;
pub const RC_INVALID_DEVICE_PROP_VALUE: u16 = 0x201C;
pub const RC_SESSION_ALREADY_OPEN: u16 = 0x201E;
pub const RC_NIKON_NOT_LIVEVIEW: u16 = 0xA00B;

// --- Événements ---------------------------------------------------------------
pub const EC_OBJECT_ADDED: u16 = 0x4002;
pub const EC_DEVICE_PROP_CHANGED: u16 = 0x4006;
pub const EC_CAPTURE_COMPLETE: u16 = 0x400D;

// --- Propriétés ---------------------------------------------------------------
pub const DPC_BATTERY_LEVEL: u16 = 0x5001;
pub const DPC_FNUMBER: u16 = 0x5007;
pub const DPC_FOCAL_LENGTH: u16 = 0x5008;
pub const DPC_EXPOSURE_TIME: u16 = 0x500D;
pub const DPC_EXPOSURE_PROGRAM: u16 = 0x500E;
pub const DPC_EXPOSURE_INDEX: u16 = 0x500F;
pub const DPC_EXPOSURE_BIAS: u16 = 0x5010;
pub const DPC_DIGITAL_ZOOM: u16 = 0x5016;

// --- Formats ------------------------------------------------------------------
pub const OFC_ASSOCIATION: u16 = 0x3001;

fn lookup(table: &[(u16, &'static str)], code: u16) -> Option<&'static str> {
    table.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
}

const OPS: &[(u16, &str)] = &[
    (0x1001, "GetDeviceInfo"),
    (0x1002, "OpenSession"),
    (0x1003, "CloseSession"),
    (0x1004, "GetStorageIDs"),
    (0x1005, "GetStorageInfo"),
    (0x1006, "GetNumObjects"),
    (0x1007, "GetObjectHandles"),
    (0x1008, "GetObjectInfo"),
    (0x1009, "GetObject"),
    (0x100A, "GetThumb"),
    (0x100B, "DeleteObject"),
    (0x100C, "SendObjectInfo"),
    (0x100D, "SendObject"),
    (0x100E, "InitiateCapture"),
    (0x100F, "FormatStore"),
    (0x1010, "ResetDevice"),
    (0x1011, "SelfTest"),
    (0x1012, "SetObjectProtection"),
    (0x1013, "PowerDown"),
    (0x1014, "GetDevicePropDesc"),
    (0x1015, "GetDevicePropValue"),
    (0x1016, "SetDevicePropValue"),
    (0x1017, "ResetDevicePropValue"),
    (0x1018, "TerminateOpenCapture"),
    (0x1019, "MoveObject"),
    (0x101A, "CopyObject"),
    (0x101B, "GetPartialObject"),
    (0x101C, "InitiateOpenCapture"),
    (0x90C0, "Nikon InitiateCaptureRecInSdram"),
    (0x90C1, "Nikon AfDrive"),
    (0x90C2, "Nikon ChangeCameraMode"),
    (0x90C3, "Nikon DelImageSDRAM"),
    (0x90C4, "Nikon GetLargeThumb"),
    (0x90C7, "Nikon CheckEvent"),
    (0x90C8, "Nikon DeviceReady"),
    (0x90C9, "Nikon SetPreWBData"),
    (0x90CA, "Nikon GetVendorPropCodes"),
    (0x90CB, "Nikon AfCaptureSDRAM"),
    (0x90CC, "Nikon GetPictCtrlData"),
    (0x90CD, "Nikon SetPictCtrlData"),
    (0x9201, "Nikon StartLiveView"),
    (0x9202, "Nikon EndLiveView"),
    (0x9203, "Nikon GetLiveViewImg"),
    (0x9204, "Nikon MfDrive"),
    (0x9205, "Nikon ChangeAfArea"),
    (0x9206, "Nikon AfDriveCancel"),
    (0x9207, "Nikon InitiateCaptureRecInMedia"),
    (0x9209, "Nikon GetVendorStorageIDs"),
    (0x920A, "Nikon StartMovieRecInCard"),
    (0x920B, "Nikon EndMovieRec"),
    (0x920C, "Nikon TerminateCapture"),
    (0x9400, "Nikon GetPartialObjectHiSpeed"),
];

const RESPONSES: &[(u16, &str)] = &[
    (0x2001, "OK"),
    (0x2002, "GeneralError"),
    (0x2003, "SessionNotOpen"),
    (0x2004, "InvalidTransactionID"),
    (0x2005, "OperationNotSupported"),
    (0x2006, "ParameterNotSupported"),
    (0x2007, "IncompleteTransfer"),
    (0x2008, "InvalidStorageId"),
    (0x2009, "InvalidObjectHandle"),
    (0x200A, "DevicePropNotSupported"),
    (0x200B, "InvalidObjectFormatCode"),
    (0x200C, "StoreFull"),
    (0x200D, "ObjectWriteProtected"),
    (0x200E, "StoreReadOnly"),
    (0x200F, "AccessDenied"),
    (0x2010, "NoThumbnailPresent"),
    (0x2011, "SelfTestFailed"),
    (0x2012, "PartialDeletion"),
    (0x2013, "StoreNotAvailable"),
    (0x2014, "SpecificationByFormatUnsupported"),
    (0x2015, "NoValidObjectInfo"),
    (0x2016, "InvalidCodeFormat"),
    (0x2017, "UnknownVendorCode"),
    (0x2018, "CaptureAlreadyTerminated"),
    (0x2019, "DeviceBusy"),
    (0x201A, "InvalidParentObject"),
    (0x201B, "InvalidDevicePropFormat"),
    (0x201C, "InvalidDevicePropValue"),
    (0x201D, "InvalidParameter"),
    (0x201E, "SessionAlreadyOpen"),
    (0x201F, "TransactionCancelled"),
    (0x2020, "SpecificationOfDestinationUnsupported"),
    (0xA001, "Nikon HardwareError"),
    (0xA002, "Nikon OutOfFocus"),
    (0xA003, "Nikon ChangeCameraModeFailed"),
    (0xA004, "Nikon InvalidStatus"),
    (0xA005, "Nikon SetPropertyNotSupported"),
    (0xA00B, "Nikon NotLiveView"),
];

const EVENTS: &[(u16, &str)] = &[
    (0x4001, "CancelTransaction"),
    (0x4002, "ObjectAdded"),
    (0x4003, "ObjectRemoved"),
    (0x4004, "StoreAdded"),
    (0x4005, "StoreRemoved"),
    (0x4006, "DevicePropChanged"),
    (0x4007, "ObjectInfoChanged"),
    (0x4008, "DeviceInfoChanged"),
    (0x4009, "RequestObjectTransfer"),
    (0x400A, "StoreFull"),
    (0x400B, "DeviceReset"),
    (0x400C, "StorageInfoChanged"),
    (0x400D, "CaptureComplete"),
    (0x400E, "UnreportedStatus"),
    (0xC101, "Nikon ObjectAddedInSDRAM"),
    (0xC102, "Nikon CaptureCompleteRecInSdram"),
    (0xC104, "Nikon AdvancedTransfer"),
    (0xC105, "Nikon PreviewImageAdded"),
];

const PROPS: &[(u16, &str)] = &[
    (0x5001, "BatteryLevel"),
    (0x5002, "FunctionalMode"),
    (0x5003, "ImageSize"),
    (0x5004, "CompressionSetting"),
    (0x5005, "WhiteBalance"),
    (0x5006, "RGBGain"),
    (0x5007, "FNumber"),
    (0x5008, "FocalLength"),
    (0x5009, "FocusDistance"),
    (0x500A, "FocusMode"),
    (0x500B, "ExposureMeteringMode"),
    (0x500C, "FlashMode"),
    (0x500D, "ExposureTime"),
    (0x500E, "ExposureProgramMode"),
    (0x500F, "ExposureIndex (ISO)"),
    (0x5010, "ExposureBiasCompensation"),
    (0x5011, "DateTime"),
    (0x5012, "CaptureDelay"),
    (0x5013, "StillCaptureMode"),
    (0x5014, "Contrast"),
    (0x5015, "Sharpness"),
    (0x5016, "DigitalZoom"),
    (0x5017, "EffectMode"),
    (0x5018, "BurstNumber"),
    (0x5019, "BurstInterval"),
    (0x501A, "TimelapseNumber"),
    (0x501B, "TimelapseInterval"),
    (0x501C, "FocusMeteringMode"),
    (0x501D, "UploadURL"),
    (0x501E, "Artist"),
    (0x501F, "CopyrightInfo"),
];

const FORMATS: &[(u16, &str)] = &[
    (0x3000, "Undefined"),
    (0x3001, "Association (dossier)"),
    (0x3006, "DPOF"),
    (0x3008, "WAV"),
    (0x300A, "AVI"),
    (0x300D, "QuickTime (MOV)"),
    (0x3801, "EXIF/JPEG"),
    (0x3808, "JFIF"),
    (0x380D, "TIFF"),
    (0xB982, "MP4"),
];

pub fn op_name(code: u16) -> Option<&'static str> {
    lookup(OPS, code)
}
pub fn response_name(code: u16) -> Option<&'static str> {
    lookup(RESPONSES, code)
}
pub fn event_name(code: u16) -> Option<&'static str> {
    lookup(EVENTS, code)
}
pub fn prop_name(code: u16) -> Option<&'static str> {
    lookup(PROPS, code)
}
pub fn format_name(code: u16) -> Option<&'static str> {
    lookup(FORMATS, code)
}

/// Motifs d'échec de la poignée de main PTP/IP (CIPA DC-X005).
pub fn init_fail_name(reason: u32) -> &'static str {
    match reason {
        1 => "initiateur refusé (GUID ou nom non autorisé)",
        2 => "appareil occupé (une autre connexion est peut-être ouverte)",
        3 => "motif non précisé",
        _ => "motif inconnu",
    }
}
