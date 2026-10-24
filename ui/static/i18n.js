// Minimal i18n layer. Every user-facing string goes through here; adding a
// string means adding it to both locales in the same change.

const DICT = {
  en: {
    "brand.sub": "studio",
    "deck.untitled": "Untitled deck",
    "deck.titleLabel": "Deck title",
    "lang.label": "Interface language",
    "lang.english": "English",
    "lang.spanish": "Spanish",
    "storyboard.title": "Storyboard",
    "storyboard.add": "Add slide",
    "storyboard.slide": "Slide {n}, {template}",
    "action.present": "Present",
    "action.undo": "Undo",
    "action.redo": "Redo",
    "action.toggleTheme": "Toggle light and dark theme",
    "action.play": "Play",
    "a11y.slideCanvas": "Current slide",
    "action.delete": "Delete slide",
    "action.copy": "Copy id",
    "stage.ready": "Ready",
    "stage.connecting": "Connecting",
    "stage.offline": "Offline",
    "empty.title": "Your story starts here",
    "empty.body": "The AI writes each slide as real visual code. Ask it for a slide and it appears here, rendered exactly as the audience will see it.",
    "empty.cta": "Add a blank slide",
    "inspector.title": "Inspector",
    "inspector.deck": "Deck",
    "inspector.theme": "Theme",
    "inspector.aspect": "Aspect ratio",
    "inspector.file": "Deck file",
    "inspector.saveCopy": "Save a copy",
    "inspector.open": "Open file…",
    "inspector.autosave": "Changes save automatically.",
    "inspector.slide": "Slide",
    "inspector.slideId": "Slide id",
    "inspector.notes": "Speaker notes",
    "inspector.notesPlaceholder": "What should you say on this slide?",
    "inspector.animation": "Animation",
    "inspector.noAnimation": "No animation steps yet. Ask the AI to animate this slide, or trigger one from the storyboard.",
    "inspector.developer": "Developer mode",
    "inspector.developerHint": "The code the AI wrote for this slide. Read-only.",
    "theme.studio": "Studio",
    "theme.paper": "Paper",
    "theme.night": "Night",
    "toast.saved": "Saved",
    "toast.opened": "Deck opened",
    "toast.downloaded": "Copy downloaded",
    "toast.error": "Something went wrong",
    "toast.copied": "Copied",
    "fatal.title": "This window is not connected",
    "fatal.body": "Open SlydeCode from the app so it can hand the editor a session token.",
    "template.hero": "Hero",
    "template.bullet": "Bullets",
    "template.diagram": "Diagram",
    "template.chart": "Chart",
    "template.image_layout": "Image",
    "template.code": "Code",
    "present.progress": "Presentation progress",
  },
  es: {
    "brand.sub": "estudio",
    "deck.untitled": "Presentación sin título",
    "deck.titleLabel": "Título de la presentación",
    "lang.label": "Idioma de la interfaz",
    "lang.english": "Inglés",
    "lang.spanish": "Español",
    "storyboard.title": "Guion visual",
    "storyboard.add": "Añadir diapositiva",
    "storyboard.slide": "Diapositiva {n}, {template}",
    "action.present": "Presentar",
    "action.undo": "Deshacer",
    "action.redo": "Rehacer",
    "action.toggleTheme": "Cambiar entre tema claro y oscuro",
    "action.play": "Reproducir",
    "a11y.slideCanvas": "Diapositiva actual",
    "action.delete": "Eliminar diapositiva",
    "action.copy": "Copiar id",
    "stage.ready": "Listo",
    "stage.connecting": "Conectando",
    "stage.offline": "Sin conexión",
    "empty.title": "Tu historia empieza aquí",
    "empty.body": "La IA escribe cada diapositiva como código visual real. Pídele una y aparecerá aquí, tal y como la verá el público.",
    "empty.cta": "Añadir diapositiva en blanco",
    "inspector.title": "Inspector",
    "inspector.deck": "Presentación",
    "inspector.theme": "Tema",
    "inspector.aspect": "Formato",
    "inspector.file": "Archivo",
    "inspector.saveCopy": "Guardar copia",
    "inspector.open": "Abrir archivo…",
    "inspector.autosave": "Los cambios se guardan solos.",
    "inspector.slide": "Diapositiva",
    "inspector.slideId": "Id de la diapositiva",
    "inspector.notes": "Notas del orador",
    "inspector.notesPlaceholder": "¿Qué vas a decir en esta diapositiva?",
    "inspector.animation": "Animación",
    "inspector.noAnimation": "Todavía no hay animaciones. Pídeselas a la IA o lánzalas desde el guion visual.",
    "inspector.developer": "Modo desarrollador",
    "inspector.developerHint": "El código que la IA escribió para esta diapositiva. Solo lectura.",
    "theme.studio": "Estudio",
    "theme.paper": "Papel",
    "theme.night": "Noche",
    "toast.saved": "Guardado",
    "toast.opened": "Presentación abierta",
    "toast.downloaded": "Copia descargada",
    "toast.error": "Algo ha salido mal",
    "toast.copied": "Copiado",
    "fatal.title": "Esta ventana no está conectada",
    "fatal.body": "Abre SlydeCode desde la aplicación para que el editor reciba un token de sesión.",
    "template.hero": "Portada",
    "template.bullet": "Viñetas",
    "template.diagram": "Diagrama",
    "template.chart": "Gráfico",
    "template.image_layout": "Imagen",
    "template.code": "Código",
    "present.progress": "Progreso de la presentación",
  },
};

const listeners = new Set();
let current = null;

function detect() {
  const stored = localStorage.getItem("slyde.lang");
  if (stored === "en" || stored === "es") return stored;
  return (navigator.language || "en").toLowerCase().startsWith("es") ? "es" : "en";
}

export function lang() {
  if (!current) current = detect();
  return current;
}

export function setLang(next) {
  if (next !== "en" && next !== "es") return;
  current = next;
  localStorage.setItem("slyde.lang", next);
  document.documentElement.lang = next;
  apply();
  listeners.forEach((fn) => fn(next));
}

export function onLangChange(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function t(key, vars) {
  const table = DICT[lang()] || DICT.en;
  let value = table[key] ?? DICT.en[key] ?? key;
  if (vars) {
    for (const [name, replacement] of Object.entries(vars)) {
      value = value.replace(`{${name}}`, String(replacement));
    }
  }
  return value;
}

export function apply(root = document) {
  document.documentElement.lang = lang();
  root.querySelectorAll("[data-i18n]").forEach((el) => {
    el.textContent = t(el.dataset.i18n);
  });
  root.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
    el.placeholder = t(el.dataset.i18nPlaceholder);
  });
  root.querySelectorAll("[data-i18n-title]").forEach((el) => {
    el.title = t(el.dataset.i18nTitle);
  });
  root.querySelectorAll("[data-i18n-aria]").forEach((el) => {
    el.setAttribute("aria-label", t(el.dataset.i18nAria));
  });
}
