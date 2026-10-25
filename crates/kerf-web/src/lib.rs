//! Kerf's engine for the version that runs in a browser.
//!
//! The page calls [`call`] with the same command names and arguments the
//! desktop app's interface sends to its own backend, so the interface is the
//! same in both. What a browser must do for itself (saving files, keeping
//! projects, asking for fonts, making web requests) the page does, with a few
//! extra commands here to help it.

use std::cell::RefCell;
use std::sync::Arc;

use base64::Engine;
use kerf_ai::{Endpoint, ImageEndpoint, Shape, ToolSpec};
use kerf_core::camera::{self, BedPictureRequest, CameraFit, CameraStill};
use kerf_core::design::Design;
use kerf_core::drawing::TextSpec;
use kerf_core::material::{self, FindMaterialRequest};
use kerf_core::{Added, DepthOptions, JobInput, LoadedImage};
use kerf_gcode::Job;
use kerf_make::{BoxSpec, LayerSpec};
use kerf_trace::image::{DynamicImage, RgbaImage};
use kerf_trace::TraceSettings;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

/// What is being worked on. A page has one thread, so one of these.
#[derive(Default)]
struct State {
    image: Option<LoadedImage>,
    /// A box or layered design, which takes the place of the traced picture.
    design: Option<Design>,
    /// Text of the most recently generated program, kept for saving.
    gcode: Option<String>,
    /// The latest picture from the camera over the bed. Kept apart from the
    /// work above: loading or clearing a picture doesn't throw it away.
    camera_still: Option<Arc<RgbaImage>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Looks at or changes the state. Never held while long work runs, so that a
/// failure in that work cannot leave it locked.
fn with<T>(work: impl FnOnce(&mut State) -> T) -> T {
    STATE.with(|state| work(&mut state.borrow_mut()))
}

fn image(preview: bool) -> Result<Arc<DynamicImage>, String> {
    with(|state| {
        let loaded = state.image.as_ref().ok_or("No image is loaded.")?;
        Ok(if preview {
            loaded.preview.clone()
        } else {
            loaded.full.clone()
        })
    })
}

fn depth_map() -> Option<Arc<DynamicImage>> {
    with(|state| state.image.as_ref().and_then(|loaded| loaded.depth.clone()))
}

fn design_svg(width_mm: f64, height_mm: f64) -> Option<String> {
    with(|state| {
        state
            .design
            .as_ref()
            .map(|design| design.to_svg(width_mm, height_mm))
    })
}

fn args<T: DeserializeOwned>(args: &str) -> Result<T, String> {
    serde_json::from_str(args).map_err(|e| format!("The request was not understood: {e}"))
}

fn reply(value: impl Serialize) -> Result<String, String> {
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

const DONE: &str = "null";

#[derive(Deserialize)]
struct Crop {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
struct Trace {
    settings: TraceSettings,
    preview: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sized {
    settings: TraceSettings,
    width_mm: f64,
    height_mm: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Generate {
    settings: TraceSettings,
    width_mm: f64,
    height_mm: f64,
    job: Job,
    depth: DepthOptions,
    added: Option<Added>,
    cutout_around: Option<[f64; 4]>,
}

#[derive(Deserialize)]
struct SetDesign {
    svg: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MakeBox {
    spec: BoxSpec,
    sheet_width: f64,
    sheet_height: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MakeLayers {
    spec: LayerSpec,
    settings: TraceSettings,
    light_is_high: bool,
    depth: DepthOptions,
    sheet_width: f64,
    sheet_height: f64,
}

#[derive(Deserialize)]
struct ReadSvg {
    svg: String,
    name: String,
}

#[derive(Deserialize)]
struct MakeText {
    spec: TextSpec,
}

#[derive(Deserialize)]
struct TurnRequest {
    endpoint: Endpoint,
    key: Option<String>,
    system: String,
    messages: Vec<Value>,
    tools: Vec<ToolSpec>,
}

#[derive(Deserialize)]
struct TurnReply {
    endpoint: Endpoint,
    reply: Value,
    tools: Vec<ToolSpec>,
    /// How many messages the conversation had when it was sent.
    sent: usize,
}

#[derive(Deserialize)]
struct ModelsRequest {
    endpoint: Endpoint,
    key: Option<String>,
}

#[derive(Deserialize)]
struct ModelsReply {
    endpoint: Endpoint,
    reply: Value,
}

#[derive(Deserialize)]
struct PictureRequest {
    endpoint: ImageEndpoint,
    key: Option<String>,
    prompt: String,
    shape: Shape,
}

#[derive(Deserialize)]
struct PictureReply {
    endpoint: ImageEndpoint,
    reply: Value,
}

#[derive(Deserialize)]
struct Failure {
    status: u16,
    body: Value,
}

#[derive(Deserialize)]
struct Unreached {
    url: String,
    why: String,
}

/// Runs one command. `input` is its arguments as JSON, and `file` the bytes
/// of the file it was given, for those that take one. The answer is JSON.
pub fn run(command: &str, input: &str, file: &[u8]) -> Result<String, String> {
    match command {
        "set_camera_still" => {
            let still = CameraStill::load(file)?;
            let (width, height) = (still.image.width(), still.image.height());
            with(|state| state.camera_still = Some(still.image));
            reply(json!({ "width": width, "height": height }))
        }
        "check_camera_fit" => {
            #[derive(Deserialize)]
            struct Input {
                fit: CameraFit,
            }
            reply(camera::check_fit(&args::<Input>(input)?.fit)?)
        }
        "find_material" => {
            let request: FindMaterialRequest = args(input)?;
            let still = with(|state| state.camera_still.clone())
                .ok_or("Take a picture with the camera first.")?;
            reply(material::find_material_request(&still, &request)?)
        }

        "app_info" => reply(json!({ "name": "Kerf", "version": env!("CARGO_PKG_VERSION") })),

        "load_image" => {
            let loaded = LoadedImage::load(file)?;
            with(|state| {
                // A new picture starts a new piece of work, but the bed hasn't
                // changed, so the camera's picture stays.
                *state = State {
                    image: Some(loaded),
                    camera_still: state.camera_still.take(),
                    ..State::default()
                }
            });
            Ok(DONE.into())
        }
        "clear_image" => {
            with(|state| {
                state.image = None;
                state.design = None;
            });
            Ok(DONE.into())
        }
        "crop_image" => {
            let Crop {
                x,
                y,
                width,
                height,
            } = args(input)?;
            with(|state| {
                state
                    .image
                    .as_mut()
                    .ok_or("No image is loaded.")?
                    .crop(x, y, width, height)
            })?;
            Ok(DONE.into())
        }
        // Takes an edited copy of the picture, such as one with its background removed.
        "replace_image" => {
            with(|state| {
                state
                    .image
                    .as_mut()
                    .ok_or("No image is loaded.")?
                    .replace(file)
            })?;
            Ok(DONE.into())
        }
        "reset_crop" => {
            with(|state| {
                state
                    .image
                    .as_mut()
                    .ok_or("No image is loaded.")
                    .map(LoadedImage::reset_crop)
            })?;
            Ok(DONE.into())
        }
        "set_depth_map" => {
            let depth = kerf_core::decode_depth(file)?;
            with(|state| {
                state.image.as_mut().ok_or("No image is loaded.")?.depth = depth;
                Ok::<_, String>(())
            })?;
            Ok(DONE.into())
        }

        "trace_image" => {
            let Trace { settings, preview } = args(input)?;
            if let Some(traced) = with(|state| state.design.as_ref().map(Design::traced)) {
                return reply(traced);
            }
            reply(kerf_trace::trace(image(preview)?.as_ref(), &settings)?)
        }
        // The traced picture, or the design, as an SVG document to save.
        "svg_text" => {
            let Sized {
                settings,
                width_mm,
                height_mm,
            } = args(input)?;
            kerf_core::check_size(width_mm, height_mm)?;
            let svg = match design_svg(width_mm, height_mm) {
                Some(svg) => svg,
                None => kerf_trace::trace(image(false)?.as_ref(), &settings)?
                    .to_svg(width_mm, height_mm),
            };
            reply(svg)
        }
        "generate_gcode" => {
            let Generate {
                settings,
                width_mm,
                height_mm,
                job,
                depth,
                added,
                cutout_around,
            } = args(input)?;
            with(|state| state.gcode = None);
            let (generated, text) = kerf_core::generate(JobInput {
                image: image(false)?,
                design_svg: design_svg(width_mm, height_mm),
                depth_map: depth_map(),
                settings,
                width_mm,
                height_mm,
                job,
                depth,
                added,
                cutout_around,
            })?;
            with(|state| state.gcode = Some(text));
            reply(generated)
        }
        // The last generated program, to save.
        "gcode_text" => reply(
            with(|state| state.gcode.clone()).ok_or("Generate the G-code before exporting it.")?,
        ),

        "set_design" => {
            let SetDesign { svg } = args(input)?;
            let design = svg.as_deref().map(Design::new).transpose()?;
            with(|state| {
                state.design = design;
                state.gcode = None;
            });
            Ok(DONE.into())
        }
        "make_box" => {
            let MakeBox {
                spec,
                sheet_width,
                sheet_height,
            } = args(input)?;
            reply(kerf_core::design::box_sheets(
                &spec,
                sheet_width,
                sheet_height,
            )?)
        }
        "make_layers" => {
            let MakeLayers {
                spec,
                settings,
                light_is_high,
                depth,
                sheet_width,
                sheet_height,
            } = args(input)?;
            let depth = if depth.enabled {
                Some(depth.source(depth_map())?)
            } else {
                None
            };
            reply(kerf_core::design::layer_sheets(
                image(false)?.as_ref(),
                depth.as_ref(),
                &settings,
                light_is_high,
                &spec,
                sheet_width,
                sheet_height,
            )?)
        }
        "read_svg" => {
            let ReadSvg { svg, name } = args(input)?;
            reply(kerf_core::drawing::drawn(&svg, &name)?)
        }
        "make_text" => {
            let MakeText { spec } = args(input)?;
            reply(kerf_core::drawing::text_made(&spec)?)
        }
        "list_fonts" => reply(kerf_gcode::font_families()),
        // Takes a font file, and answers with the families that were in it.
        "add_font" => reply(kerf_gcode::add_font(file.to_vec())),

        // The assistant. The page makes each request and hands back the reply.
        "turn_request" => {
            let TurnRequest {
                endpoint,
                key,
                system,
                messages,
                tools,
            } = args(input)?;
            reply(kerf_ai::turn_request(
                &endpoint,
                key.as_deref(),
                &system,
                &messages,
                &tools,
            )?)
        }
        "turn_reply" => {
            let TurnReply {
                endpoint,
                reply: answer,
                tools,
                sent,
            } = args(input)?;
            reply(kerf_ai::read_turn(&endpoint, &answer, &tools, sent)?)
        }
        "models_request" => {
            let ModelsRequest { endpoint, key } = args(input)?;
            reply(kerf_ai::models_request(&endpoint, key.as_deref())?)
        }
        "models_reply" => {
            let ModelsReply {
                endpoint,
                reply: answer,
            } = args(input)?;
            reply(kerf_ai::read_models(&endpoint, &answer)?)
        }
        "picture_request" => {
            let PictureRequest {
                endpoint,
                key,
                prompt,
                shape,
            } = args(input)?;
            reply(kerf_ai::image_request(
                &endpoint,
                key.as_deref(),
                &prompt,
                shape,
            ))
        }
        // Answers with the picture as base64, or the address to collect it from.
        "picture_reply" => {
            let PictureReply {
                endpoint,
                reply: answer,
            } = args(input)?;
            reply(match kerf_ai::read_image(&endpoint, &answer)? {
                kerf_ai::Generated::Bytes(bytes) => {
                    json!({ "base64": base64::engine::general_purpose::STANDARD.encode(bytes) })
                }
                kerf_ai::Generated::Url(url) => json!({ "url": url }),
            })
        }
        "describe_failure" => {
            let Failure { status, body } = args(input)?;
            reply(kerf_ai::describe_failure(status, &body))
        }
        "could_not_reach" => {
            let Unreached { url, why } = args(input)?;
            reply(kerf_ai::could_not_reach(&url, &why))
        }

        _ => Err(format!("This is not available in the browser: {command}")),
    }
}

/// One of the pictures being worked on as a PNG file, for saving a project:
/// `image` or `depth`. No bytes at all means there is no such picture.
pub fn picture_png(which: &str) -> Result<Vec<u8>, String> {
    let picture = with(|state| {
        let loaded = state.image.as_ref().ok_or("There is no picture to save.")?;
        match which {
            "image" => Ok(Some(loaded.full.clone())),
            "depth" => Ok(loaded.depth.clone()),
            _ => Err("Unknown picture.".to_string()),
        }
    })?;
    match picture {
        Some(picture) => kerf_core::encode_png(&picture),
        None => Ok(Vec::new()),
    }
}

/// The latest camera still redrawn to the bed, as a PNG file. `input` is a
/// [`BedPictureRequest`] as JSON.
pub fn camera_picture_png(input: &str) -> Result<Vec<u8>, String> {
    let request: BedPictureRequest = args(input)?;
    let still =
        with(|state| state.camera_still.clone()).ok_or("Take a picture with the camera first.")?;
    camera::camera_bed_picture_png(&still, &request)
}

/// Makes failures inside the engine show in the browser's console.
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

/// [`run`], for the page to call.
#[wasm_bindgen]
pub fn call(command: &str, input: &str, file: &[u8]) -> Result<String, JsError> {
    run(command, input, file).map_err(|e| JsError::new(&e))
}

/// [`camera_picture_png`], for the page to call.
#[wasm_bindgen]
pub fn camera_picture(input: &str) -> Result<Vec<u8>, JsError> {
    camera_picture_png(input).map_err(|e| JsError::new(&e))
}

/// [`picture_png`], for the page to call.
#[wasm_bindgen]
pub fn picture(which: &str) -> Result<Vec<u8>, JsError> {
    picture_png(which).map_err(|e| JsError::new(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kerf_trace::image::{GrayImage, ImageFormat, Luma};
    use std::io::Cursor;

    const SETTINGS: &str = r#"{"mode":"outline","preprocess":{"brightness":0,"contrast":0,"blurSigma":0,"threshold":{"mode":"otsu"},"invert":false},"speckle":4,"curves":true,"cornerThreshold":60,"colorPrecision":6,"layerDifference":16}"#;
    const MACHINE: &str = r#"{"kind":"laser","origin":"frontLeft","startGcode":"","endGcode":"","bedWidth":400,"bedHeight":400,"maxPowerS":1000,"travelFeed":6000}"#;

    /// A black square on white, as a PNG file.
    fn square() -> Vec<u8> {
        let picture = GrayImage::from_fn(120, 120, |x, y| {
            Luma([if (30..90).contains(&x) && (30..90).contains(&y) {
                0
            } else {
                255
            }])
        });
        let mut file = Cursor::new(Vec::new());
        picture.write_to(&mut file, ImageFormat::Png).unwrap();
        file.into_inner()
    }

    fn job() -> String {
        format!(
            r#"{{"settings":{SETTINGS},"widthMm":60,"heightMm":60,"job":{{"machine":{MACHINE},"offset":[10,10],"operations":[{{"color":null,"kind":{{"type":"laserCut"}},"feed":1000,"power":50,"passes":1}}]}},"depth":{{"enabled":false,"detail":0.15,"floor":0.1}},"added":null,"cutoutAround":null}}"#
        )
    }

    #[test]
    fn a_picture_goes_from_file_to_gcode() {
        assert!(run("gcode_text", "{}", &[]).is_err());
        run("load_image", "", &square()).unwrap();

        let traced: Value = serde_json::from_str(
            &run(
                "trace_image",
                &format!(r#"{{"settings":{SETTINGS},"preview":true}}"#),
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(traced["width"], 120);
        assert!(traced["pathCount"].as_u64().unwrap() >= 1);

        let generated: Value =
            serde_json::from_str(&run("generate_gcode", &job(), &[]).unwrap()).unwrap();
        assert!(generated["lineCount"].as_u64().unwrap() > 4);
        assert!(generated["head"].as_str().unwrap().contains("G21"));
        let text: String = serde_json::from_str(&run("gcode_text", "{}", &[]).unwrap()).unwrap();
        assert_eq!(
            text.lines().count() as u64,
            generated["lineCount"].as_u64().unwrap()
        );

        // An edited copy of the same size can take its place; another size cannot.
        run("replace_image", "", &square()).unwrap();
        assert!(run("replace_image", "", b"not a picture").is_err());

        // The picture comes back out as a PNG, and there is no depth map.
        assert!(picture_png("image")
            .unwrap()
            .starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(picture_png("depth").unwrap().is_empty());

        run("clear_image", "{}", &[]).unwrap();
        assert!(run(
            "trace_image",
            &format!(r#"{{"settings":{SETTINGS},"preview":true}}"#),
            &[]
        )
        .is_err());
    }

    #[test]
    fn a_box_needs_no_picture() {
        let sheets: Value = serde_json::from_str(
            &run(
                "make_box",
                r#"{"spec":{"width":100,"depth":80,"height":60,"thickness":3,"fingerWidth":0,"lid":true,"cutWidth":0.1,"dogbones":false},"sheetWidth":400,"sheetHeight":400}"#,
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        let sheets = sheets.as_array().unwrap();
        assert!(!sheets.is_empty());
        assert!(sheets[0]["svg"].as_str().unwrap().starts_with("<svg"));
    }

    #[test]
    fn the_assistants_requests_are_handed_to_the_page() {
        let request: Value = serde_json::from_str(
            &run(
                "turn_request",
                r#"{"endpoint":{"protocol":"openAi","baseUrl":"https://openrouter.ai/api/v1","model":"m"},"key":"k","system":"s","messages":[{"role":"user","content":"hi"}],"tools":[]}"#,
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            request["url"],
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(request["body"]["model"], "m");

        let turn: Value = serde_json::from_str(
            &run(
                "turn_reply",
                r#"{"endpoint":{"protocol":"openAi","baseUrl":"https://openrouter.ai/api/v1","model":"m"},"reply":{"model":"m","choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"Hello"}}]},"tools":[],"sent":1}"#,
                &[],
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(turn["text"], "Hello");
        assert_eq!(turn["stop"]["type"], "done");
    }

    #[test]
    fn unknown_commands_say_so() {
        assert!(run("connect_machine", "{}", &[])
            .unwrap_err()
            .contains("not available in the browser"));
    }

    #[test]
    fn the_camera_still_survives_loading_a_new_picture() {
        let size = run("set_camera_still", "{}", &square()).unwrap();
        assert_eq!(size, r#"{"height":120,"width":120}"#);
        run("load_image", "{}", &square()).unwrap();
        run("clear_image", "{}", &[]).unwrap();

        let request = r#"{"fit":{"points":[{"px":0,"py":0,"x":0,"y":0},{"px":120,"py":0,"x":60,"y":0},{"px":120,"py":120,"x":60,"y":60},{"px":0,"py":120,"x":0,"y":60}],"bedWidth":60,"bedHeight":60,"pictureWidth":120,"pictureHeight":120},"bed":{"width":60,"height":60}}"#;
        let png = camera_picture_png(request).unwrap();
        let picture = kerf_trace::image::load_from_memory(&png).unwrap();
        assert_eq!((picture.width(), picture.height()), (120, 120));
    }

    #[test]
    fn the_material_is_found_in_the_camera_still() {
        let request = r#"{"fit":{"points":[{"px":0,"py":0,"x":0,"y":0},{"px":120,"py":0,"x":60,"y":0},{"px":120,"py":120,"x":60,"y":60},{"px":0,"py":120,"x":0,"y":60}],"bedWidth":60,"bedHeight":60,"pictureWidth":120,"pictureHeight":120},"bed":{"width":60,"height":60},"seed":null,"stock":null,"thickness":3}"#;
        assert!(run("find_material", request, &[])
            .unwrap_err()
            .contains("Take a picture"));

        // The dark square, 30 mm across in the middle of a 60 mm bed, is the material.
        run("set_camera_still", "{}", &square()).unwrap();
        let found: Value =
            serde_json::from_str(&run("find_material", request, &[]).unwrap()).unwrap();
        assert!(
            (found["width"].as_f64().unwrap() - 30.0).abs() < 1.0,
            "{found}"
        );
        assert!(
            (found["centre"][0].as_f64().unwrap() - 30.0).abs() < 1.0,
            "{found}"
        );
        assert_eq!(found["corners"].as_array().unwrap().len(), 4);
        assert!(found["warnings"].as_array().unwrap().is_empty(), "{found}");
    }
}
