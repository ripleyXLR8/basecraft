//! Script steps as FileMaker shows them in the Script Workspace:
//! `Set Field [ Contacts::Name ; "x" ]`, `If [ … ]`, `# comment`,
//! disabled steps prefixed with `//`, and the indentation of blocks.

use engine::Database;
use fmp12::scripts::{
    Calc, FieldRef, LayoutDest, RecordDest, Script, SortOrder, Step, StepParams, Target, id,
};

/// Names of fields, occurrences, layouts and scripts for step text, and
/// the reverse lookup for compiling calculations typed by the user.
pub struct Names<'a> {
    pub db: &'a Database,
    pub scripts: &'a [Script],
}

impl fmp12::calc::Names for Names<'_> {
    fn field(&self, to: u32, field: u32) -> Option<String> {
        Some(self.db.table_of(to)?.field(field)?.name.clone())
    }
    fn occurrence(&self, to: u32) -> Option<String> {
        Some(
            self.db
                .occurrences
                .iter()
                .find(|o| o.id == to)?
                .name
                .clone(),
        )
    }
}

impl fmp12::calc::Resolve for Names<'_> {
    fn occurrences(&self) -> Vec<(u32, String)> {
        self.db
            .occurrences
            .iter()
            .map(|o| (o.id, o.name.clone()))
            .collect()
    }
    fn fields(&self, to: u32) -> Vec<(u32, String)> {
        self.db.table_of(to).map_or_else(Vec::new, |t| {
            t.fields.iter().map(|f| (f.id, f.name.clone())).collect()
        })
    }
}

impl Names<'_> {
    /// Source text of a calculation (`<error>` if it does not decode).
    pub fn calc(&self, c: &Calc) -> String {
        c.text(self).unwrap_or_else(|_| "<error>".into())
    }

    pub fn field_ref(&self, f: &FieldRef) -> String {
        let to = fmp12::calc::Names::occurrence(self, f.to).unwrap_or_else(|| "?".into());
        let name = fmp12::calc::Names::field(self, f.to, f.field).unwrap_or_else(|| "?".into());
        if f.repetition > 1 {
            format!("{to}::{name}[{}]", f.repetition)
        } else {
            format!("{to}::{name}")
        }
    }

    pub fn layout(&self, id: u32) -> String {
        self.db
            .layouts
            .iter()
            .find(|l| l.id == id)
            .map_or_else(|| "<unknown>".into(), |l| l.name.clone())
    }

    pub fn script(&self, id: u32) -> String {
        self.scripts
            .iter()
            .find(|s| s.id == id)
            .map_or_else(|| "<unknown>".into(), |s| s.name.clone())
    }

    /// Compiles `src` (script context: field references name their TO).
    pub fn compile(&self, src: &str) -> Result<Calc, fmp12::calc::CompileError> {
        fmp12::calc::compile(src, 0, self).map(|c| Calc(c.tokens))
    }
}

/// The step as one line of FileMaker step text (no indentation).
pub fn step_text(s: &Step, n: &Names) -> String {
    step_text_in(s, n, false)
}

/// [`step_text`] as FileMaker's French UI shows it when `fr`: French step
/// names and labels, `Nom [a; b]` instead of `Name [ a ; b ]`.
/// Calculations keep their English function names.
pub fn step_text_in(s: &Step, n: &Names, fr: bool) -> String {
    let w = |en: &'static str| if fr { label_fr(en) } else { en };
    let on = |b: bool| w(if b { "On" } else { "Off" });
    let name = if fr { step_name_fr(s.id) } else { s.name() }
        .map_or_else(|| format!("<unknown> [{}]", s.id), str::to_string);
    let c = |c: &Option<Calc>| c.as_ref().map(|c| n.calc(c)).unwrap_or_default();
    let mut args: Vec<String> = Vec::new();
    match &s.params {
        StepParams::Comment(t) => {
            let t = if t.is_empty() {
                String::new()
            } else {
                format!("# {t}")
            };
            return if s.enabled { t } else { format!("// {t}") };
        }
        StepParams::Switch(b) => args.push(on(*b).into()),
        StepParams::Condition(x) => args.push(c(x)),
        StepParams::PerformScript { script, parameter } => {
            args.push(format!(
                "\u{201C}{}\u{201D}",
                script.map_or_else(|| "<unknown>".into(), |i| n.script(i))
            ));
            if parameter.is_some() {
                args.push(format!("{}{}", w("Parameter: "), c(parameter)));
            }
        }
        StepParams::ExitScript(x) => {
            if x.is_some() {
                args.push(format!("{}{}", w("Text Result: "), c(x)));
            }
        }
        StepParams::SetVariable {
            name,
            value,
            repetition,
        } => {
            let rep = repetition
                .as_ref()
                .map(|r| n.calc(r))
                .filter(|r| r != "1")
                .map(|r| format!("[{r}]"))
                .unwrap_or_default();
            args.push(format!("{name}{rep}"));
            args.push(format!("{}{}", w("Value: "), c(value)));
        }
        StepParams::SetField { field, value, .. } => {
            if let Some(f) = field {
                args.push(n.field_ref(f));
            }
            args.push(c(value));
        }
        StepParams::InsertText {
            target,
            text,
            select,
        } => {
            if *select {
                args.push(w("Select").into());
            }
            match target {
                Target::Field(f) => args.push(format!("{}{}", w("Target: "), n.field_ref(f))),
                Target::Variable(v) => args.push(format!("{}{v}", w("Target: "))),
                Target::None => {}
            }
            args.push(if fr {
                format!("\u{201C}{text}\u{201D}")
            } else {
                format!("\"{text}\"")
            });
        }
        StepParams::GoToLayout { dest, .. } => args.push(match dest {
            LayoutDest::Original => w("original layout").into(),
            LayoutDest::Layout(l) => format!("\u{201C}{}\u{201D}", n.layout(*l)),
            LayoutDest::NameByCalc(x) => format!("{}{}", w("Name by calculation: "), n.calc(x)),
            LayoutDest::NumberByCalc(x) => {
                format!("{}{}", w("Number by calculation: "), n.calc(x))
            }
        }),
        StepParams::GoToRecord {
            dest,
            exit_after_last,
            dialog,
        } => {
            match dest {
                RecordDest::First => args.push(w("First").into()),
                RecordDest::Last => args.push(w("Last").into()),
                RecordDest::Previous => args.push(w("Previous").into()),
                RecordDest::Next => args.push(w("Next").into()),
                RecordDest::ByCalc(x) => {
                    args.push(format!("{}{}", w("With dialog: "), on(*dialog)));
                    args.push(n.calc(x));
                }
            }
            if matches!(dest, RecordDest::Previous | RecordDest::Next) {
                args.push(format!(
                    "{}{}",
                    w("Exit after last: "),
                    on(*exit_after_last)
                ));
            }
        }
        StepParams::GoToField { field, select } => {
            if *select {
                args.push(w("Select/perform").into());
            }
            if let Some(f) = field {
                args.push(n.field_ref(f));
            }
        }
        StepParams::GoToObject { name, repetition } => {
            args.push(format!("{}{}", w("Object Name: "), c(name)));
            if repetition.is_some() {
                args.push(format!("{}{}", w("Repetition: "), c(repetition)));
            }
        }
        StepParams::ShowCustomDialog {
            title,
            message,
            input,
            ..
        } => {
            args.push(c(title));
            args.push(c(message));
            if let Some((f, _)) = input {
                args.push(n.field_ref(f));
            }
        }
        StepParams::Records { dialog, count } => {
            if s.id != id::NEW_RECORD && s.id != id::DUPLICATE_RECORD {
                args.push(format!("{}{}", w("With dialog: "), on(*dialog)));
            }
            if count.is_some() {
                args.push(c(count));
            }
        }
        StepParams::Commit {
            dialog,
            skip_validation,
            ..
        } => {
            if *skip_validation {
                args.push(w("Skip data entry validation").into());
            }
            args.push(format!("{}{}", w("With dialog: "), on(*dialog)));
        }
        StepParams::Find {
            pause,
            restore,
            requests,
        } => {
            if s.id == id::ENTER_FIND_MODE {
                args.push(format!("{}{}", w("Pause: "), on(*pause)));
            }
            if *restore && !requests.is_empty() {
                args.push(w("Restore").into());
            }
        }
        StepParams::Mode { pause } => args.push(format!("{}{}", w("Pause: "), on(*pause))),
        StepParams::Sort {
            dialog,
            restore,
            keys,
        } => {
            if *restore && !keys.is_empty() {
                args.push(w("Restore").into());
            }
            args.push(format!("{}{}", w("With dialog: "), on(*dialog)));
        }
        StepParams::RefreshWindow { flush_joins, .. } => {
            if *flush_joins {
                args.push(w("Flush cached join results").into());
            }
        }
        StepParams::Pause(d) => args.push(match d {
            Some(d) => format!("{}{}", w("Duration (seconds): "), n.calc(d)),
            None => w("Indefinitely").into(),
        }),
        StepParams::None | StepParams::Other => {}
    }
    let text = if args.is_empty() {
        name
    } else if fr {
        format!("{name} [{}]", args.join("; "))
    } else {
        format!("{name} [ {} ]", args.join(" ; "))
    };
    if s.enabled {
        text
    } else {
        format!("// {text}")
    }
}

/// Labels of the step text in FileMaker 26's French UI, as it shows them in
/// the Script Workspace (read from its window through UI Automation).
fn label_fr(en: &'static str) -> &'static str {
    match en {
        "On" => "Oui",
        "Off" => "Non",
        "Parameter: " => "Paramètre:",
        "Text Result: " => "Texte de résultat:",
        "Value: " => "Valeur: ",
        "Select" => "Sélectionner",
        "Target: " => "Cible:",
        "original layout" => "modèle d'origine",
        // FileMaker shows the calculation alone.
        "Name by calculation: " | "Number by calculation: " => "",
        "First" => "Premièr(e)",
        "Last" => "Dernièr(e)",
        "Previous" => "Précédent(e)",
        "Next" => "Suivant(e)",
        "With dialog: " => "Avec boîte de dialogue:",
        "Exit after last: " => "Sortie après dernière:",
        "Select/perform" => "Sélectionner/exécuter",
        "Object Name: " => "Nom d'objet : ",
        "Repetition: " => "Valeur nº : ",
        "Skip data entry validation" => "Ignorer la validation lors de la saisie des données",
        "Pause: " => "Pause:",
        "Restore" => "Rétablir",
        "Flush cached join results" => "Vider résultats de jointure en mémoire cache",
        "Duration (seconds): " => "Durée (secondes) : ",
        "Indefinitely" => "Indéfiniment",
        other => other,
    }
}

/// Step names in FileMaker 26's French UI (Script Workspace step list and
/// palette, read through UI Automation), by step id. Some steps keep their
/// English name there.
pub fn step_name_fr(id: u16) -> Option<&'static str> {
    Some(match id {
        89 => "# (commentaire)",
        1 => "Exécuter script",
        3 => "Enregistrer en XML",
        4 => "Rubrique suivante",
        5 => "Rubrique précédente",
        6 => "Activer modèle",
        7 => "Nouvel enreg./requête",
        8 => "Dupliquer enreg./requête",
        9 => "Supprimer enreg./requête",
        10 => "Supprimer tous enreg.",
        11 => "Insérer à partir de l'index",
        12 => "Insérer depuis dern. consulté",
        13 => "Insérer date du jour",
        14 => "Insérer heure actuelle",
        16 => "Afficher enreg/requête/page",
        17 => "Activer rubrique",
        18 => "Vérifier sélection",
        19 => "Vérifier enregistrement",
        20 => "Vérifier enreg. trouvés",
        21 => "Annuler tri des enreg.",
        22 => "Mode Recherche",
        23 => "Afficher tous les enreg.",
        24 => "Modifier la recherche",
        25 => "Ignorer enregistrement",
        26 => "Ignorer plusieurs enreg.",
        27 => "Afficher enreg. ignorés uniquement",
        28 => "Exécuter la recherche",
        29 => "Afficher/masquer barres d'outils",
        30 => "Format affichage",
        31 => "Ajuster fenêtre",
        32 => "Ouvrir Aide",
        33 => "Ouvrir fichier",
        34 => "Fermer fichier",
        35 => "Importer enregistrements",
        36 => "Exporter enregistrements",
        37 => "Enregistrer une copie",
        38 => "Ouvrir Gérer la base de données",
        39 => "Trier enregistrements",
        40 => "Mettre à jour contenu rubrique",
        41 => "Mode Prévisualisation",
        42 => "Configuration impression",
        43 => "Imprimer",
        44 => "Quitter application",
        45 => "Annuler/Rétablir",
        46 => "Couper",
        47 => "Copier",
        48 => "Coller",
        49 => "Effacer",
        50 => "Sélectionner tout",
        51 => "Rétablir enreg./requête",
        55 => "Mode Utilisation",
        56 => "Insérer image",
        57 => "Emettre événement",
        59 => "Insérer QuickTime",
        60 => "Insérer nom de l'utilisateur actuel",
        61 => "Insérer texte",
        62 => "Suspendre/reprendre script",
        63 => "Envoyer courrier",
        64 => "Emettre Exécution DDE",
        65 => "Composer numéro",
        66 => "Prononcer",
        67 => "Exécuter AppleScript",
        68 => "Si",
        69 => "Sinon",
        70 => "Fin de si",
        71 => "Boucle",
        72 => "Fin de boucle si",
        73 => "Fin de boucle",
        74 => "Activer enregistrements liés",
        75 => "Valider enreg./requêtes",
        76 => "Définir rubrique",
        77 => "Insérer résultat du calcul",
        78 => "Insérer objet",
        79 => "Figer fenêtre",
        80 => "Rafraîchir fenêtre",
        81 => "Faire défiler fenêtre",
        82 => "Nouveau fichier",
        83 => "Modifier mot de passe",
        84 => "Multi-utilisateur",
        85 => "Autoriser l'annulation par l'utilisateur",
        86 => "Gestion erreurs",
        87 => "Ouvrir boîte dial. person.",
        88 => "Ouvrir l’espace de travail de script",
        90 => "Interrompre script",
        91 => "Remplacer contenu rubrique",
        92 => "Afficher/masquer réglette texte",
        93 => "Signal sonore",
        94 => "Formats du système",
        95 => "Récupérer fichier",
        96 => "Enregistrer une copie en tant que module d'extension",
        97 => "Agrandir/réduire",
        98 => "Copier tous enreg./requêtes",
        99 => "Activer rangée externe",
        101 => "Copier enreg./requête",
        102 => "Vider mémoire cache",
        103 => "Fin de script",
        104 => "Supprimer rangée externe",
        105 => "Ouvrir les réglages",
        106 => "Orthographes proposées",
        107 => "Options d'orthographe",
        108 => "Sélectionner des dictionnaires",
        109 => "Dictionnaire utilisateur",
        111 => "Ouvrir URL",
        112 => "Ouvrir Gérer les listes de valeurs",
        113 => "Ouvrir Partage",
        114 => "Ouvrir Options fichier",
        115 => "Autoriser barre de texte",
        116 => "Définir valeur en série suiv.",
        117 => "Exécuter SQL",
        118 => "Ouvrir Hôtes",
        119 => "Déplacer/redimensionner fenêtre",
        120 => "Organiser toutes les fenêtres",
        121 => "Fermer fenêtre",
        122 => "Nouvelle fenêtre",
        123 => "Sélectionner fenêtre",
        124 => "Définir titre de fenêtre",
        125 => "Sinon si",
        126 => "Restreindre ensemble trouvé",
        127 => "Développer ensemble trouvé",
        128 => "Rechercher et remplacer",
        129 => "Ouvrir Rech. et remplacer",
        130 => "Définir sélection",
        131 => "Insérer fichier",
        132 => "Exporter contenu de rubrique",
        133 => "Ouvrir enreg./requête",
        134 => "Ajouter compte",
        135 => "Supprimer compte",
        136 => "Réinit. mot de passe compte",
        137 => "Activer compte",
        138 => "Reconnexion",
        139 => "Convertir fichier",
        140 => "Ouvrir Gérer les sources de données",
        141 => "Définir variable",
        142 => "Installer jeu de menus",
        143 => "Sauvegarder en Excel",
        144 => "Sauvegarder en PDF",
        145 => "Aller à l'objet",
        146 => "Définir Web Viewer",
        147 => "Définir rubrique par nom",
        148 => "Installer un script sur temporisation",
        149 => "Ouvrir Modifier les recherches enregistrées",
        150 => "Exécuter une recherche rapide",
        151 => "Ouvrir Gérer les modèles",
        152 => "Enregistrer en lien snapshot",
        154 => "Trier enreg. par rubrique",
        155 => "Rech. enreg. concordants",
        156 => "Ouvrir Gérer les conteneurs",
        157 => "Installer le fichier de plug-in",
        158 => "Insérer PDF",
        159 => "Insérer audio/vidéo",
        160 => "Insérer depuis URL",
        161 => "Insérer depuis appareil",
        164 => "Exécuter script sur serveur",
        165 => "Ouvrir Gérer les thèmes",
        166 => "Afficher/masquer barre de menu",
        167 => "Actualiser objet",
        168 => "Activer les animations d'objets",
        169 => "Fermer Popover",
        172 => "Ouvrir Upload vers hôte",
        174 => "Activer le clavier tactile",
        175 => "Exécuter JavaScript dans Web Viewer",
        176 => "Définir les orientations autorisées",
        177 => "Lire AVPlayer",
        178 => "Régler l'état de lecture AVPlayer",
        179 => "Définir options AVPlayer",
        180 => "Actualiser la table externe",
        181 => "Obtenir le chemin du dossier",
        182 => "Tronquer la table",
        183 => "Ouvrir Favoris",
        184 => "Open Starter Solution",
        185 => "Configurer un script de surveillance de zone",
        187 => "Configurer une notification locale",
        188 => "Vérifier l'existence du fichier",
        189 => "Obtenir la taille du fichier",
        190 => "Créer un fichier de données",
        191 => "Ouvrir un fichier de données",
        192 => "Écrire dans le fichier de données",
        193 => "Lire depuis le fichier de données",
        194 => "Obtenir la position dans le fichier de données",
        195 => "Définir la position dans le fichier de données",
        196 => "Fermer le fichier de données",
        197 => "Supprimer le fichier",
        199 => "Renommer le fichier",
        200 => "Activer la journalisation des erreurs",
        201 => "Configurer lecture du protocole NFC",
        202 => "Configurer le modèle de Machine Learning",
        203 => "Exécuter FileMaker Data API",
        205 => "Ouvrir une transaction",
        206 => "Valider la transaction",
        207 => "Annuler la transaction",
        208 => "Définir un identifiant de session",
        209 => "Définir le dictionnaire",
        210 => "Exécuter script sur serveur avec rappel",
        211 => "Déclencher un flux Claris Connect",
        212 => "Configurer le compte IA",
        213 => "Peaufiner le modèle",
        214 => "Exécuter une requête SQL en langage naturel",
        215 => "Insérer l'intégration",
        216 => "Insérer l'intégration dans le jeu trouvé",
        217 => "Définir la journalisation des appels d'IA",
        218 => "Effectuer une recherche sémantique",
        219 => "Exécuter une action RAG",
        220 => "Générer une réponse à partir du modèle",
        221 => "Effectuer une recherche en langage naturel",
        222 => "Configurer un modèle de régression",
        223 => "Définir Annuler la transaction sur erreur",
        224 => "Execute URL",
        225 => "Sauvegarder les enregistrements au format JSONL",
        226 => "Configurer un modèle de prompt",
        227 => "Configurer un compte RAG",
        228 => "Accéder à la liste des enregistrements",
        237 => "Purger les cookies du Web Viewer",
        238 => "Configurer les données persistantes",
        239 => "Validate Scripts",
        240 => "Insérer des légendes d'images dans le jeu trouvé",
        241 => "Insérer une légende d'image",
        242 => "Imprimer le PDF",
        243 => "Créer un PDF",
        244 => "Ajouter au PDF",
        245 => "Fermer le PDF",
        246 => "Ouvrir un PDF",
        247 => "Annuler le PDF",
        _ => return None,
    })
}

/// Indentation level of each step: one more inside If/Else If/Else and
/// Loop blocks.
pub fn indents(steps: &[Step]) -> Vec<usize> {
    let mut depth: usize = 0;
    steps
        .iter()
        .map(|s| match s.id {
            id::ELSE_IF | id::ELSE => depth.saturating_sub(1),
            id::END_IF | id::END_LOOP => {
                depth = depth.saturating_sub(1);
                depth
            }
            id::IF | id::LOOP => {
                depth += 1;
                depth - 1
            }
            _ => depth,
        })
        .collect()
}

/// Sort keys as text, for the Sort Records editor.
pub fn sort_key_text(k: &(FieldRef, SortOrder), n: &Names) -> String {
    let o = match k.1 {
        SortOrder::Ascending => "ascending",
        SortOrder::Descending => "descending",
        SortOrder::Custom(_) => "custom",
    };
    format!("{} ({o})", n.field_ref(&k.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indentation() {
        let st = |i: u16| Step {
            id: i,
            enabled: true,
            ..Default::default()
        };
        let steps: Vec<Step> = [
            id::IF,
            id::BEEP,
            id::ELSE_IF,
            id::LOOP,
            id::BEEP,
            id::END_LOOP,
            id::ELSE,
            id::END_IF,
            id::BEEP,
        ]
        .into_iter()
        .map(st)
        .collect();
        assert_eq!(indents(&steps), [0, 1, 0, 1, 2, 1, 0, 0, 0]);
    }
}
