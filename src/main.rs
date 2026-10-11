mod adjunto;
mod enlace;
mod ask;
mod baymax;
mod buddy;
mod claude;
mod clip;
mod commands;
mod entrada;
mod habla;
mod estilo;
mod md;
mod musica;
mod surco;
mod miniatura;
mod nucleo;
mod select;
mod sessions;
mod sixel;
mod teclado;
mod theme;
mod ui;
mod rutas;
mod update;
mod voice;
mod app;
mod eventos;
mod teclas;
mod raton;
mod ordenes;
mod enviar;
mod escucha;
mod flotante;
mod pantalla;

use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};

use ask::{Ask, Outcome};
use nucleo::{Event as Gesto, State};
use claude::{Claude, ClaudeEvent};
use voice::{Level, VoiceCmd, VoiceEvent};

use app::*;
use teclas::*;
use raton::*;
use ordenes::*;
use enviar::*;
use escucha::*;
use flotante::*;
use pantalla::*;

fn main() -> Result<()> {
    rutas::migrar();
    voice::prefer_discrete_gpu();
    let mut extra: Vec<String> = std::env::args().skip(1).collect();
    if extra.first().map(String::as_str) == Some("--transcribe") {
        return voice::transcribe_file(extra.get(1).map_or("", String::as_str));
    }
    // --flotante: la ventana que se llama con un atajo, te escucha sola, ejecuta y se esconde.
    let flotante = extra.iter().any(|a| a == "--flotante");
    extra.retain(|a| a != "--flotante");
    if flotante {
        extra.extend(["--append-system-prompt".into(), FLOTANTE_PROMPT.into()]);
    }
    let (tx, rx) = mpsc::channel();

    let mut claude = Some(Claude::spawn(&extra, tx.clone())?);
    let (voice_tx, level) = voice::spawn(tx.clone());
    update::spawn(tx.clone());

    // `iris --resume <id>`: además de pasárselo al motor, se muestra la conversación.
    let resumed = extra.iter().position(|a| a == "--resume").and_then(|i| extra.get(i + 1)).map(|id| sessions::replay(id));
    let (messages, activity, agents_done) = resumed.map(|r| (r.messages, r.activity, r.agents)).unwrap_or_default();
    let turn_from = activity.len();
    let mut app = App {
        messages,
        activity,
        input: String::new(),
        scroll: 0,
        busy: false,
        thinking: false,
        voice: VoiceState::Loading,
        voice_model: String::new(),
        levels: vec![0.0; 256],
        model: String::new(),
        effort: "auto".into(),
        session: String::new(),
        cost: 0.0,
        turns: 0,
        started: Instant::now(),
        alive: true,
        commands: commands::load(),
        menu: 0,
        modal: None,
        interrupted: false,
        assistant_open: false,
        level,
        key_release: false,
        space_down: false,
        last_space: Instant::now(),
        released: None,
        ptt: None,
        view: Default::default(),
        sel: None,
        flash: None,
        nucleo: nucleo::Core::new(),
        ctx_used: 0,
        ctx_window: 200_000,
        todos: (0, 0),
        compacting: false,
        last_activity: Instant::now(),
        quit_armed: None,
        update: None,
        reexec: false,
        last_key: Instant::now(),
        last_voice: Instant::now(),
        noise_floor: 0.0,
        booted: Instant::now(),
        calm: rutas::hay_var("CALM"),
        md_cache: Default::default(),
        adjuntos_cache: Default::default(),
        images: vec![],
        cur: 0,
        historial: entrada::Historial::load(),
        flotante,
        focused: true,
        hands_free: None,
        heard: false,
        hide_at: None,
        habla: habla::Habla::new(tx.clone()),
        lector: habla::Lector::new(),
        voz_modo: match rutas::var("HABLA").as_deref() {
            _ if flotante => VozModo::Siempre,
            Some("always") | Some("siempre") => VozModo::Siempre,
            Some("never") | Some("nunca") | Some("0") => VozModo::Nunca,
            _ => VozModo::Auto,
        },
        spoken_next: false,
        speak_turn: false,
        talking: false,
        tts_level: 0.0,
        musica: musica::Musica::spawn(),
        surco: surco::Surco::spawn(),
        music_at: std::cell::Cell::new(None),
        estilo: estilo::cargar(),
        buddy: buddy::cargar(),
        reading: Default::default(),
        transcript: false,
        tools_view: None,
        agents_tab: false,
        tools_view_max: Default::default(),
        read_override: None,
        read_top: Default::default(),
        read_msg: std::cell::Cell::new(usize::MAX),
        sixel_cell: sixel_cell(),
        core_rect: Default::default(),
        thumb_slots: Default::default(),
        file_slots: Default::default(),
        file_targets: Default::default(),
        thumb_targets: Default::default(),
        thumbs_shown: vec![],
        sixel_shown: None,
        turn_from,
        agents: vec![],
        agents_done,
        next_kid: 0,
    };

    let mut term = ratatui::init();
    // Para mantener espacio y hablar hace falta saber cuándo se suelta. Con «desambiguar» y
    // «tipos de evento» las letras siguen llegando como texto (acentos y tildes muertas
    // intactos) y además llega el soltar. «Todas las teclas como códigos» duplica la é en foot.
    use crossterm::event::{KeyboardEnhancementFlags as K, PushKeyboardEnhancementFlags};
    if crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
        let flags = K::DISAMBIGUATE_ESCAPE_CODES | K::REPORT_EVENT_TYPES;
        app.key_release = crossterm::execute!(std::io::stdout(), PushKeyboardEnhancementFlags(flags)).is_ok();
    }
    // Con el mouse en manos de Iris, arrastrar copia solo texto del chat; Shift+arrastrar
    // sigue siendo la selección de la terminal.
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture);
    // Lo pegado llega en un solo evento: así un texto de varias líneas no se envía en el
    // primer salto, y una ruta de imagen arrastrada a la terminal se puede adjuntar.
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste);
    // Saber si la ventana tiene el foco: para avisar con una notificación y, en el modo
    // flotante, para empezar a escuchar al aparecer.
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::EnableFocusChange);
    let res = run(&mut term, &mut app, &mut claude, &voice_tx, &tx, &rx, &extra);
    if app.key_release {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::event::PopKeyboardEnhancementFlags);
    }
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableFocusChange);
    ratatui::restore();
    if res.is_ok() && app.reexec {
        // Se instaló la versión nueva: se reabre con ella y retoma esta misma sesión.
        drop(claude);
        let mut args: Vec<String> = vec![];
        let mut old = std::env::args().skip(1);
        while let Some(a) = old.next() {
            if a == "--resume" {
                old.next();
            } else {
                args.push(a);
            }
        }
        if !app.session.is_empty() {
            args.extend(["--resume".into(), app.session.clone()]);
        }
        drop(app);
        // En Linux la versión nueva toma el lugar de esta; Windows no tiene exec, así que la
        // abre, la espera y sale con lo que ella devuelva.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let err = std::process::Command::new(update::exe()).args(args).exec();
            return Err(err.into());
        }
        #[cfg(not(unix))]
        {
            let status = std::process::Command::new(update::exe()).args(args).status()?;
            std::process::exit(status.code().unwrap_or(0));
        }
    }
    res
}

fn run(
    term: &mut ratatui::DefaultTerminal,
    app: &mut App,
    claude: &mut Option<Claude>,
    voice_tx: &Sender<VoiceCmd>,
    tx: &Sender<AppEvent>,
    rx: &mpsc::Receiver<AppEvent>,
    extra: &[String],
) -> Result<()> {
    // 60 fps: lo que más se mueve es el núcleo.
    let tick = Duration::from_millis(16);
    let mut frame = Instant::now();
    let mut frames: u64 = 0;
    let mut teclado = teclado::Teclado::new();
    let mut theme_check = Instant::now();
    theme::poll();
    // Los colores con que se dibujó el markdown guardado. /buddy, /core y sus vistas
    // previas los cambian sin pasar por `poll`, así que se compara en cada vuelta.
    let mut colores = (theme::acento_u32(), theme::texto_u32(), theme::fondo_rgb());
    loop {
        // El tema puede cambiar en cualquier momento (Omarchy, Aether): se mira una vez por segundo.
        if theme_check.elapsed() >= Duration::from_secs(1) {
            theme_check = Instant::now();
            theme::poll();
        }
        let ahora = (theme::acento_u32(), theme::texto_u32(), theme::fondo_rgb());
        if ahora != colores {
            colores = ahora;
            app.md_cache.borrow_mut().clear(); // el markdown guardado lleva los colores viejos
        }
        // La onda avanza aunque no haya audio, para que se vea viva.
        let lvl = if app.voice == VoiceState::Listening { voice::level_get(&app.level) } else { 0.0 };
        app.levels.remove(0);
        app.levels.push(lvl);
        if app.voice == VoiceState::Listening {
            hear(app, lvl);
        }
        flotante_tick(app, voice_tx);
        // La música de surco calla mientras escucha, transcribe, piensa lo que va a decir o habla.
        let quiet = matches!(app.voice, VoiceState::Listening | VoiceState::Transcribing)
            || app.talking
            || (app.busy && app.speak_turn);
        app.surco.quiet(quiet);
        // La versión nueva se ofrece cuando no estorba: si apareciera mientras escribes, el
        // Enter de tu mensaje la aceptaría.
        if app.update.is_some()
            && app.modal.is_none()
            && app.tools_view.is_none()
            && app.input.is_empty()
            && !app.busy
            && app.last_key.elapsed() > Duration::from_secs(3)
        {
            app.modal = app.update.take().map(Modal::Update);
        }

        let dt = frame.elapsed().as_secs_f64();
        frame = Instant::now();
        // Los que terminaron se quedan unos segundos en ACTIVIDAD y luego se van.
        // Los que terminaron hace rato dejan el núcleo, pero quedan para verlos en Ctrl+G.
        let (viejos, vivos): (Vec<Agent>, Vec<Agent>) =
            std::mem::take(&mut app.agents).into_iter().partition(|a| a.ended.is_some_and(|(t, _)| t.elapsed() >= Duration::from_secs(4)));
        app.agents = vivos;
        app.agents_done.extend(viejos);
        let sobra = app.agents_done.len().saturating_sub(40);
        app.agents_done.drain(..sobra);
        let (want, sig) = (app.state(), app.signals());
        app.nucleo.step(dt, want, &sig);
        // El teclado acompaña al núcleo: su estado, la voz y los eventos.
        teclado.tick(app.nucleo.state(), sig.level);
        for ev in app.nucleo.take_fired() {
            teclado.event(ev);
        }

        app.core_rect.set(None);
        app.thumb_slots.borrow_mut().clear();
        app.thumb_targets.borrow_mut().clear();
        app.file_slots.borrow_mut().clear();
        app.file_targets.borrow_mut().clear();
        // Lo que se dibujó en este cuadro: hace falta para reescribir el texto que tapaba una
        // miniatura que se movió.
        let snap = term.draw(|f| ui::draw(f, app))?.buffer.clone();
        if sixel_frame(term, app, &mut frames)? {
            app.thumbs_shown.clear(); // la pantalla se limpió: hay que volver a dibujarlas
        }
        thumbs_frame(term, app, &snap)?;
        follow_drag(app);

        if event::poll(tick)? {
            let ev = event::read()?;
            if let Event::Mouse(m) = ev {
                on_mouse(app, m);
            }
            match ev {
                Event::FocusGained => {
                    app.focused = true;
                    app.hide_at = None;
                    if app.flotante && app.voice == VoiceState::Ready && !app.busy && !app.talking {
                        let _ = voice_tx.send(VoiceCmd::Start);
                        app.hands_free = Some(Instant::now());
                        app.heard = false;
                    }
                }
                Event::FocusLost => app.focused = false,
                _ => {}
            }
            if let Event::Paste(text) = &ev {
                app.last_activity = Instant::now();
                pasted(app, text);
            }
            if let Event::Key(k) = ev {
                let space = k.code == KeyCode::Char(' ');
                if k.kind == KeyEventKind::Release && space {
                    app.space_down = false;
                    app.released = Some(Instant::now());
                }
                // Las repeticiones sirven para borrar o moverse con la tecla apretada.
                if k.kind == KeyEventKind::Press || (k.kind == KeyEventKind::Repeat && !space) {
                    match handle_key(app, k, claude, voice_tx) {
                        Flow::Quit => return Ok(()),
                        Flow::Restart => {
                            *claude = None;
                            app.drop_waiting();
                            *claude = Some(Claude::spawn(extra, tx.clone())?);
                            app.effort = "auto".into();
                            app.alive = true;
                            app.modal = None;
                            app.ctx_used = 0;
                            app.todos = (0, 0);
                            app.booted = Instant::now();
                            app.nucleo.reboot();
                            app.agents.clear();
                            app.push(Role::System, "claude reiniciado — sesión nueva");
                        }
                        Flow::NewChat => {
                            *claude = None;
                            app.drop_waiting();
                            *claude = Some(Claude::spawn(extra, tx.clone())?);
                            app.effort = "auto".into();
                            app.alive = true;
                            app.busy = false;
                            app.modal = None;
                            app.messages.clear();
                            app.md_cache.borrow_mut().clear();
                            app.adjuntos_cache.borrow_mut().clear();
                            app.activity.clear();
                            app.ctx_used = 0;
                            app.todos = (0, 0);
                            app.booted = Instant::now();
                            app.nucleo.reboot();
                            app.agents.clear();
                            app.push(Role::System, "conversación nueva · la anterior se retoma con /resume");
                            term.clear()?;
                        }
                        Flow::Resume(id) => {
                            let mut args = extra.to_vec();
                            args.extend(["--resume".into(), id.clone()]);
                            *claude = None;
                            *claude = Some(Claude::spawn(&args, tx.clone())?);
                            app.effort = "auto".into();
                            app.alive = true;
                            app.busy = false;
                            app.modal = None;
                            app.activity.clear();
                            app.todos = (0, 0);
                            app.booted = Instant::now();
                            app.nucleo.reboot();
                            app.agents.clear();
                            // Chat, herramientas y subagentes de la sesión: Ctrl+G los vuelve a ver.
                            let replay = sessions::replay(&id);
                            app.messages = replay.messages;
                            app.activity = replay.activity;
                            app.agents_done = replay.agents;
                            app.turn_from = app.activity.len();
                            app.md_cache.borrow_mut().clear();
                            app.adjuntos_cache.borrow_mut().clear();
                            app.push(Role::System, format!("sesión {} retomada", &id[..8]));
                            term.clear()?;
                        }
                        Flow::Redraw => term.clear()?,
                        Flow::Update => {
                            app.push(Role::System, "actualizando: bajando la versión nueva…");
                            update::install(tx.clone());
                        }
                        Flow::Go => {}
                    }
                }
            }
        }

        if app.released.is_some_and(|t| t.elapsed() >= REPEAT_GAP) {
            app.released = None;
            space_up(app, voice_tx);
        }

        while let Ok(ev) = rx.try_recv() {
            match ev {
                AppEvent::Claude(id, e) if claude.as_ref().is_some_and(|c| c.id == id) => {
                    app.on_claude(e)
                }
                AppEvent::Claude(..) => {}
                AppEvent::Voice(e) => on_voice(app, e, claude),
                AppEvent::Habla(e) => match e {
                    habla::HablaEvent::Start => app.talking = true,
                    habla::HablaEvent::Level(l) => app.tts_level = l,
                    habla::HablaEvent::Idle => {
                        app.talking = false;
                        app.tts_level = 0.0;
                        if app.flotante && !app.busy {
                            app.hide_at = Some(Instant::now() + Duration::from_secs(3));
                        }
                    }
                    habla::HablaEvent::Unavailable(why) => {
                        app.push(Role::Error, format!("sin voz: {why}"));
                    }
                },
                AppEvent::Update(e) => match e {
                    update::UpdateEvent::Available(info) => app.update = Some(info),
                    update::UpdateEvent::Installed => {
                        app.reexec = true;
                        return Ok(());
                    }
                    update::UpdateEvent::Failed(why) => {
                        app.push(Role::Error, format!("no se pudo actualizar: {why}"));
                    }
                },
            }
        }
    }
}

enum Flow {
    Go,
    /// `/clear`: conversación nueva, como en Claude Code. La anterior queda guardada.
    NewChat,
    Redraw,
    Quit,
    Restart,
    Resume(String),
    /// Bajar e instalar la versión nueva.
    Update,
}
