//! Opening, importing and saving a model from and as bytes, for platforms where files have no
//! path (a browser): the same outcomes as `open_file`, `import_file` and `save_file`.

use super::*;

impl Editor {
    /// Opens a `.skr` file, or a SketchUp file as a new model never saved, from its `bytes`
    /// (`name` is the file's name), dropping unsaved changes (the window asks about them
    /// first). False if it cannot be opened; the status then says why.
    pub fn open_bytes(&mut self, name: &str, bytes: &[u8]) -> bool {
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        self.measure = None;
        let opened = match self.doc.open_bytes(name, bytes, true) {
            Ok(report) => {
                self.documents += 1;
                self.material = None;
                if let Some(report) = report {
                    self.message = Some(Message::info(format!(
                        "Opened {name}{}",
                        sketchup_summary(&report)
                    )));
                }
                true
            }
            Err(e) => {
                self.message = Some(Message::error(e.message));
                false
            }
        };
        self.invalidate_hover();
        opened
    }

    /// Imports an OBJ, STL or SketchUp file from its `bytes` as `import_file` does the file
    /// (`name` gives the format and the group's name). An OBJ's material libraries are not
    /// read. False if it cannot be imported; the status then says why.
    pub fn import_bytes(&mut self, name: &str, bytes: Vec<u8>) -> bool {
        self.reset_tool();
        self.typed.clear();
        self.message = None;
        let sketchup = is_sketchup_path(Path::new(name));
        let options = self.import;
        let given = session::ImportOptions {
            place: options.place,
            scale: (!sketchup && options.units != ImportUnits::Meters)
                .then(|| options.units.meters()),
            up: if sketchup { None } else { options.up },
            merge_coplanar: (!sketchup).then_some(options.merge_coplanar),
            ..Default::default()
        };
        let result = self.doc.import_bytes(name, bytes, &given);
        self.invalidate_hover();
        match result {
            Ok(outcome) => {
                let Some(report) = outcome.import else {
                    return false;
                };
                self.say_imported(name, sketchup, &report);
                true
            }
            Err(e) => {
                self.message = Some(Message::error(e.message));
                false
            }
        }
    }

    /// The model as the bytes of a `.skr` file; the model stays as it is until
    /// `saved_as` says they were kept. None if it cannot be encoded; the status then says why.
    pub fn encode_model(&mut self) -> Option<Vec<u8>> {
        self.message = None;
        match self.doc.encode_bytes() {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                self.message = Some(Message::error(e.message));
                None
            }
        }
    }

    /// The bytes of the model exported in `format` (the extension of one of
    /// `export_formats`), with the files beside it that an OBJ names, named after `stem`. None
    /// if it cannot be exported; the status then says why.
    pub fn export_model(&mut self, format: &str, stem: &str) -> Option<Vec<(String, Vec<u8>)>> {
        self.message = None;
        let Some(format) = export_format(format) else {
            self.message = Some(Message::error(format!("Cannot export as .{format}")));
            return None;
        };
        match self.doc.export_bytes(format, stem, None) {
            Ok(exported) => {
                let ext = format_extension(format);
                let mut message = format!("Exported {stem}.{ext}");
                match exported.report.skipped_faces {
                    0 => {}
                    1 => message += " without 1 face that could not be triangulated",
                    n => message += &format!(" without {n} faces that could not be triangulated"),
                }
                if !exported.report.warnings.is_empty() {
                    message = format!("{message} ({})", exported.report.warnings.join("; "));
                }
                self.message = Some(Message::info(message));
                self.last_export = ext;
                let mut files = vec![(format!("{stem}.{ext}"), exported.bytes)];
                files.extend(exported.side_files);
                Some(files)
            }
            Err(e) => {
                self.message = Some(Message::error(e.message));
                None
            }
        }
    }

    /// The model's bytes were kept under `name`: no unsaved changes, and the status says so.
    pub fn saved_as(&mut self, name: &str) {
        self.doc.mark_saved();
        self.message = Some(Message::info(format!("Saved {name}")));
    }
}

fn export_format(extension: &str) -> Option<session::ExportFormat> {
    use session::ExportFormat as F;
    Some(match extension {
        "obj" => F::Obj,
        "stl" => F::Stl,
        "glb" => F::Glb,
        "gltf" => F::Gltf,
        "skp" => F::Skp,
        "svg" => F::Svg,
        _ => return None,
    })
}

fn format_extension(format: session::ExportFormat) -> &'static str {
    use session::ExportFormat as F;
    match format {
        F::Obj => "obj",
        F::Stl => "stl",
        F::Glb => "glb",
        F::Gltf => "gltf",
        F::Skp => "skp",
        F::Svg => "svg",
    }
}
