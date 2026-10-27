// Complete starter decks, ready to edit.
//
// Each template is a story, not a pile of layouts: every slide has a job (the
// problem, the proof, the ask) and sample words that say what to write there.
// Numbers are deliberately placeholders ("00%"): a template must never pass off
// invented data as real. Content exists in English and Spanish and is rendered
// through the same layouts the Layout card offers, so a template slide can be
// re-laid-out afterwards without losing a word.

import { renderLayout } from "./layouts.js";

const slide = (layout, en, es, notes) => ({ layout, content: { en, es }, notes });

const text = (title, subtitle = "", extra = {}) => ({
  eyebrow: "",
  title,
  subtitle,
  paragraphs: [],
  bullets: [],
  image: "",
  ...extra,
});

export const TEMPLATES = [
  {
    id: "startup_pitch",
    theme: "studio",
    name: { en: "Startup pitch", es: "Pitch de startup" },
    description: {
      en: "Problem, solution, proof and the ask, in seven slides.",
      es: "Problema, solución, prueba y petición, en siete diapositivas.",
    },
    title: { en: "Startup pitch", es: "Pitch de startup" },
    slides: [
      slide(
        "hero",
        text("Your company name", "One sentence that says what you do and for whom.", { eyebrow: "Pitch" }),
        text("El nombre de tu empresa", "Una frase que diga qué haces y para quién.", { eyebrow: "Pitch" }),
        { en: "Open with the sentence you would say to a stranger in a lift.", es: "Empieza con la frase que le dirías a un desconocido en un ascensor." }
      ),
      slide(
        "bullet",
        text("The problem", "", { bullets: ["Who feels it, and how often", "What it costs them today", "Why the current options fall short"] }),
        text("El problema", "", { bullets: ["Quién lo sufre y con qué frecuencia", "Lo que le cuesta hoy", "Por qué las opciones actuales se quedan cortas"] }),
        { en: "Make the audience feel the problem before you mention your product.", es: "Haz que el público sienta el problema antes de nombrar tu producto." }
      ),
      slide(
        "two_columns",
        text("Our solution", "", { paragraphs: ["What it does, in plain words.", "The one thing it does better than anything else.", "How a customer uses it in under a minute.", "What changes for them afterwards."] }),
        text("Nuestra solución", "", { paragraphs: ["Qué hace, en palabras sencillas.", "Lo único que hace mejor que nadie.", "Cómo lo usa un cliente en menos de un minuto.", "Qué cambia para él después."] }),
        { en: "Show, do not list. If you can demo it, demo it.", es: "Muestra, no enumeres. Si puedes hacer una demo, hazla." }
      ),
      slide(
        "big_number",
        text("Add your key metric", "", { paragraphs: ["00%"], eyebrow: "Traction" }),
        text("Añade tu métrica clave", "", { paragraphs: ["00%"], eyebrow: "Tracción" }),
        { en: "One number, with its context: growth, retention, revenue. Replace the placeholder with a real figure.", es: "Un solo número, con su contexto: crecimiento, retención, ingresos. Sustituye el marcador por una cifra real." }
      ),
      slide(
        "bullet",
        text("How we make money", "", { bullets: ["Who pays", "How much, and how often", "Why it grows with the customer"] }),
        text("Cómo ganamos dinero", "", { bullets: ["Quién paga", "Cuánto y con qué frecuencia", "Por qué crece con el cliente"] }),
        { en: "Keep the model simple enough to explain in one breath.", es: "Mantén el modelo lo bastante simple como para explicarlo de un tirón." }
      ),
      slide(
        "quote",
        text("Customer name, role", "", { paragraphs: ["A sentence a real customer said about the difference you made."] }),
        text("Nombre del cliente, cargo", "", { paragraphs: ["Una frase real de un cliente sobre la diferencia que has marcado."] }),
        { en: "Use a real quote with permission. Nothing sells like a customer.", es: "Usa una cita real y con permiso. Nada vende como un cliente." }
      ),
      slide(
        "hero",
        text("What we are asking for", "The amount, what it unlocks and by when.", { eyebrow: "The ask" }),
        text("Lo que pedimos", "La cantidad, lo que desbloquea y para cuándo.", { eyebrow: "La petición" }),
        { en: "End with a clear request and a next step.", es: "Termina con una petición clara y un siguiente paso." }
      ),
    ],
  },
  {
    id: "product_launch",
    theme: "night",
    name: { en: "Product launch", es: "Lanzamiento de producto" },
    description: {
      en: "Announce something new: why now, what it does, where it goes next.",
      es: "Anuncia algo nuevo: por qué ahora, qué hace y hacia dónde va.",
    },
    title: { en: "Product launch", es: "Lanzamiento de producto" },
    slides: [
      slide("hero", text("Introducing Product", "The headline benefit in one line.", { eyebrow: "Launch" }), text("Presentamos Producto", "El beneficio principal en una línea.", { eyebrow: "Lanzamiento" }), { en: "Start with the benefit, not the feature.", es: "Empieza por el beneficio, no por la función." }),
      slide("section", text("Why now", "What changed in the world that makes this possible.", { eyebrow: "01" }), text("Por qué ahora", "Qué ha cambiado en el mundo para que esto sea posible.", { eyebrow: "01" }), { en: "Set the scene in two sentences.", es: "Sitúa el contexto en dos frases." }),
      slide("bullet", text("What it does", "", { bullets: ["The first thing people notice", "The feature they will tell friends about", "The detail that shows we care"] }), text("Qué hace", "", { bullets: ["Lo primero que se nota", "La función que contarán a sus amigos", "El detalle que demuestra que nos importa"] }), { en: "Three points are plenty.", es: "Tres puntos son más que suficientes." }),
      slide("image_layout", text("See it in action", "Add a screenshot or photo of the product.", {}), text("Míralo en acción", "Añade una captura o una foto del producto.", {}), { en: "Paste an image onto this slide to replace the placeholder.", es: "Pega una imagen en esta diapositiva para sustituir el marcador." }),
      slide("big_number", text("Launch goal", "", { paragraphs: ["00"], eyebrow: "Target" }), text("Objetivo del lanzamiento", "", { paragraphs: ["00"], eyebrow: "Objetivo" }), { en: "State the number you will judge the launch by.", es: "Indica el número con el que juzgarás el lanzamiento." }),
      slide("bullet", text("What comes next", "", { bullets: ["This month", "This quarter", "This year"] }), text("Qué viene después", "", { bullets: ["Este mes", "Este trimestre", "Este año"] }), { en: "A roadmap people can believe is short and dated.", es: "Una hoja de ruta creíble es corta y tiene fechas." }),
      slide("hero", text("Thank you", "Where to try it and how to reach us.", { eyebrow: "Product" }), text("Gracias", "Dónde probarlo y cómo contactarnos.", { eyebrow: "Producto" }), { en: "Leave the link on screen during questions.", es: "Deja el enlace en pantalla durante las preguntas." }),
    ],
  },
  {
    id: "class_lesson",
    theme: "paper",
    name: { en: "Class lesson", es: "Clase o lección" },
    description: {
      en: "Goals, key ideas, an example to remember and practice.",
      es: "Objetivos, ideas clave, un ejemplo para recordar y práctica.",
    },
    title: { en: "Lesson", es: "Lección" },
    slides: [
      slide("hero", text("Lesson title", "Course or subject · date", { eyebrow: "Today" }), text("Título de la lección", "Asignatura o curso · fecha", { eyebrow: "Hoy" }), { en: "Greet the class and say what today is about.", es: "Saluda a la clase y di de qué trata la sesión de hoy." }),
      slide("bullet", text("By the end you will be able to…", "", { bullets: ["Explain the main idea in your own words", "Solve a typical problem", "Spot the common mistake"] }), text("Al terminar podrás…", "", { bullets: ["Explicar la idea principal con tus palabras", "Resolver un problema típico", "Detectar el error más común"] }), { en: "Learning goals, written as things students can do.", es: "Objetivos de aprendizaje, escritos como cosas que el alumnado podrá hacer." }),
      slide("two_columns", text("Key ideas", "", { paragraphs: ["First idea, explained simply.", "A short example that makes it concrete.", "Second idea and how it connects to the first.", "What people usually get wrong."] }), text("Ideas clave", "", { paragraphs: ["Primera idea, explicada con sencillez.", "Un ejemplo corto que la haga concreta.", "Segunda idea y cómo se relaciona con la primera.", "Lo que suele salir mal."] }), { en: "Pause after each idea and ask a question.", es: "Haz una pausa tras cada idea y lanza una pregunta." }),
      slide("quote", text("Source or author", "", { paragraphs: ["The sentence you want everyone to remember."] }), text("Fuente o autor", "", { paragraphs: ["La frase que quieres que todos recuerden."] }), { en: "A single line to take home.", es: "Una sola frase para llevarse a casa." }),
      slide("bullet", text("Your turn", "", { bullets: ["Exercise 1: warm-up", "Exercise 2: apply it", "Exercise 3: challenge"] }), text("Ahora tú", "", { bullets: ["Ejercicio 1: calentamiento", "Ejercicio 2: aplícalo", "Ejercicio 3: reto"] }), { en: "Give a time limit for each.", es: "Pon un tiempo límite a cada uno." }),
      slide("section", text("Recap", "Three things to remember.", { eyebrow: "End" }), text("Repaso", "Tres cosas para recordar.", { eyebrow: "Fin" }), { en: "Ask students to say the recap back to you.", es: "Pide al alumnado que repita el repaso." }),
    ],
  },
  {
    id: "quarterly_report",
    theme: "pitch",
    name: { en: "Quarterly report", es: "Informe trimestral" },
    description: {
      en: "Results, wins and challenges, and next quarter's priorities.",
      es: "Resultados, logros y retos, y prioridades del próximo trimestre.",
    },
    title: { en: "Quarterly report", es: "Informe trimestral" },
    slides: [
      slide("hero", text("Quarter results", "Team or department · period", { eyebrow: "Report" }), text("Resultados del trimestre", "Equipo o departamento · periodo", { eyebrow: "Informe" }), { en: "State the headline in the first slide.", es: "Da el titular desde la primera diapositiva." }),
      slide("big_number", text("Headline figure", "", { paragraphs: ["00%"], eyebrow: "Result" }), text("Cifra principal", "", { paragraphs: ["00%"], eyebrow: "Resultado" }), { en: "The single number leadership asks about first. Replace the placeholder.", es: "El número por el que más preguntará la dirección. Sustituye el marcador." }),
      slide("two_columns", text("Highlights and challenges", "", { paragraphs: ["What went well, and why.", "What we learned from it.", "What did not go as planned.", "What we are doing about it."] }), text("Logros y retos", "", { paragraphs: ["Lo que salió bien y por qué.", "Lo que aprendimos de ello.", "Lo que no salió como esperábamos.", "Qué estamos haciendo al respecto."] }), { en: "Be as open about challenges as about wins.", es: "Sé tan claro con los retos como con los logros." }),
      slide("bullet", text("Priorities next quarter", "", { bullets: ["Priority one, with an owner", "Priority two, with an owner", "Priority three, with an owner"] }), text("Prioridades del próximo trimestre", "", { bullets: ["Prioridad uno, con responsable", "Prioridad dos, con responsable", "Prioridad tres, con responsable"] }), { en: "Fewer priorities, clearly owned.", es: "Menos prioridades, con responsable claro." }),
      slide("quote", text("Customer or colleague", "", { paragraphs: ["A real comment that shows the impact of the work."] }), text("Cliente o compañero", "", { paragraphs: ["Un comentario real que muestre el impacto del trabajo."] }), { en: "Put a human voice next to the numbers.", es: "Pon una voz humana junto a los números." }),
      slide("hero", text("Questions", "Where to find the full data.", { eyebrow: "Discussion" }), text("Preguntas", "Dónde encontrar los datos completos.", { eyebrow: "Debate" }), { en: "Leave time for discussion.", es: "Deja tiempo para el debate." }),
    ],
  },
  {
    id: "workshop",
    theme: "academic",
    name: { en: "Workshop", es: "Taller" },
    description: {
      en: "An agenda, hands-on parts and takeaways for a working session.",
      es: "Agenda, partes prácticas y conclusiones para una sesión de trabajo.",
    },
    title: { en: "Workshop", es: "Taller" },
    slides: [
      slide("hero", text("Workshop title", "Facilitator · date · place", { eyebrow: "Workshop" }), text("Título del taller", "Facilitador · fecha · lugar", { eyebrow: "Taller" }), { en: "Welcome people and set the tone.", es: "Da la bienvenida y marca el tono." }),
      slide("bullet", text("Agenda", "", { bullets: ["00:00 Welcome and goals", "00:15 Part one", "00:45 Exercise", "01:15 Takeaways"] }), text("Agenda", "", { bullets: ["00:00 Bienvenida y objetivos", "00:15 Primera parte", "00:45 Ejercicio", "01:15 Conclusiones"] }), { en: "Replace the times with yours.", es: "Sustituye las horas por las tuyas." }),
      slide("section", text("Part one", "The idea we will work with.", { eyebrow: "01" }), text("Primera parte", "La idea con la que vamos a trabajar.", { eyebrow: "01" }), { en: "Keep the explanation short; the exercise is the point.", es: "Explica poco: lo importante es el ejercicio." }),
      slide("two_columns", text("The exercise", "", { paragraphs: ["What to do, step by step.", "How long you have.", "What to produce.", "How we will share it."] }), text("El ejercicio", "", { paragraphs: ["Qué hacer, paso a paso.", "De cuánto tiempo dispones.", "Qué producir.", "Cómo lo compartiremos."] }), { en: "Make the instructions fit on the screen.", es: "Que las instrucciones quepan en pantalla." }),
      slide("bullet", text("Takeaways", "", { bullets: ["What we learned", "What to try on Monday", "Where to go deeper"] }), text("Conclusiones", "", { bullets: ["Lo que hemos aprendido", "Qué probar el lunes", "Dónde profundizar"] }), { en: "Ask each person to name one takeaway.", es: "Pide a cada persona que nombre una conclusión." }),
      slide("hero", text("Thank you", "Slides, notes and contact.", { eyebrow: "Workshop" }), text("Gracias", "Diapositivas, notas y contacto.", { eyebrow: "Taller" }), { en: "Share the materials link.", es: "Comparte el enlace a los materiales." }),
    ],
  },
];

const TEMPLATE_LABELS = { hero: "hero", bullet: "bullet", image_layout: "image_layout" };

/**
 * A template as the studio sends it: title, theme, ratio and slides with their
 * code already rendered for the canvas.
 * @param {typeof TEMPLATES[number]} template
 * @param {"en" | "es"} lang
 * @param {{w: number, h: number}} size canvas size
 */
export function buildTemplate(template, lang, size, aspectRatio = "16:9") {
  const pick = (value) => value[lang] || value.en;
  return {
    title: pick(template.title),
    theme_id: template.theme,
    aspect_ratio: aspectRatio,
    slides: template.slides.map((item) => ({
      code: renderLayout(item.layout, pick(item.content), size),
      template: TEMPLATE_LABELS[item.layout] || "custom",
      notes: pick(item.notes),
      transition: "fade",
    })),
  };
}
