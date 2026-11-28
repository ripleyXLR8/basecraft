//! UI strings. English keys; French when `BASECRAFT_LANG` or `LANG` starts
//! with "fr" (desktop) or when the browser language is French (web).

use std::sync::OnceLock;

thread_local! {
    /// Edit › Preferences › Language: `None` follows the system.
    static LANGUAGE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Sets the interface language: `Some(true)` French, `Some(false)`
/// English, `None` the system's. For the UI thread (where egui draws).
pub fn set_french(fr: Option<bool>) {
    LANGUAGE.with(|l| l.set(fr));
}

/// The system's language is French.
pub fn system_french() -> bool {
    static FR: OnceLock<bool> = OnceLock::new();
    *FR.get_or_init(|| {
        #[cfg(target_arch = "wasm32")]
        if let Some(l) = web_sys::window().and_then(|w| w.navigator().language()) {
            return l.to_lowercase().starts_with("fr");
        }
        ["BASECRAFT_LANG", "LANG", "LC_ALL"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .any(|v| v.to_lowercase().starts_with("fr"))
    })
}

/// The interface is in French (the preference, else the system's language).
pub fn french() -> bool {
    LANGUAGE.with(|l| l.get()).unwrap_or_else(system_french)
}

pub fn tr(key: &'static str) -> &'static str {
    if !french() {
        return key;
    }
    match key {
        "File" => "Fichier",
        "Edit" => "Edition",
        "View" => "Affichage",
        "Insert" => "Insertion",
        "Format" => "Format",
        "Records" => "Enregistrements",
        "Scripts" => "Scripts",
        "Tools" => "Outils",
        "Window" => "Fenêtre",
        "New Window" => "Nouvelle fenêtre",
        "Minimize Window" => "Réduire la fenêtre",
        "Tile Horizontally" => "Juxtaposition horizontale",
        "Tile Vertically" => "Juxtaposition verticale",
        "Cascade" => "Superposer",
        "Preview Mode" => "Mode Prévisualisation",
        "Count" => "Nombre",
        "Help" => "Aide",
        "Open…" => "Ouvrir…",
        "Close" => "Fermer",
        "Exit" => "Quitter",
        "Not implemented yet" => "Pas encore implémenté",
        "Picture or File…" => "Image ou fichier…",
        "Pictures" => "Images",
        "Insert Picture or File…" => "Insérer une image ou un fichier…",
        "Export Field Contents…" => "Exporter le contenu de la rubrique…",
        "Total" => "Total",
        "Unsorted" => "Non triés",
        "Show All" => "Afficher tout",
        "New Record" => "Nouvel enregistrement",
        "Delete Record" => "Supprimer l'enregistrement",
        "Find" => "Rechercher",
        "Sort" => "Trier",
        "Share" => "Partager",
        "Layout:" => "Modèle :",
        "View As:" => "Format affichage :",
        "Form" => "Formulaire",
        "List" => "Liste",
        "Table" => "Tableau",
        "Preview" => "Prévisualisation",
        "Edit Layout" => "Modifier modèle",
        "Browse Mode" => "Mode Utilisation",
        "Find Mode" => "Mode Recherche",
        "Delete Record…" => "Supprimer l'enregistrement…",
        "Show All Records" => "Afficher tous les enregistrements",
        "Sorted" => "Triés",
        "Perform Find" => "Lancer la recherche",
        "Cancel" => "Annuler",
        "Unsaved changes" => "Modifications non enregistrées",
        "Save" => "Enregistrer",
        "Data Viewer" => "Visualiseur de données",
        "Error" => "Erreur",
        "Expression" => "Expression",
        "Value" => "Valeur",
        "Add" => "Ajouter",
        "Calculation, e.g. Get ( FoundCount )" => "Calcul, p. ex. Get ( FoundCount )",
        "Page" => "Page",
        "Exit Preview" => "Quitter la prévisualisation",
        "No scripts" => "Aucun script",
        "Script error" => "Erreur de script",
        "Show Omitted Only" => "Afficher enreg. ignorés uniq.",
        "Omit Record" => "Ignorer l'enregistrement",
        "Request" => "Requête",
        "New Request" => "Nouvelle requête",
        "Omit" => "Ignorer",
        "must not be empty." => "ne doit pas être vide.",
        "must contain a unique value." => "doit contenir une valeur unique.",
        "must contain an existing value." => "doit contenir une valeur existante.",
        "must contain a number." => "doit contenir un nombre.",
        "must contain a date with a four-digit year." => {
            "doit contenir une date avec une année à quatre chiffres."
        }
        "must contain a time of day." => "doit contenir une heure du jour.",
        "must contain a value in the range" => "doit contenir une valeur comprise entre",
        "does not pass its validation calculation." => "ne satisfait pas son calcul de validation.",
        "must contain a value from the value list." => {
            "doit contenir une valeur de la liste de valeurs."
        }
        "must contain at most this many characters:" => {
            "doit contenir au plus ce nombre de caractères :"
        }
        "Accept anyway" => "Accepter quand même",
        "Save a Copy As…" => "Enregistrer une copie sous…",
        "Layout Mode" => "Mode Modèle",
        "Exit Layout" => "Quitter le modèle",
        "New Layout/Report" => "Nouveau modèle/rapport",
        "Layout Setup…" => "Configuration du modèle…",
        "Layout Setup" => "Configuration du modèle",
        "Layout Name" => "Nom du modèle",
        "Show records from" => "Afficher les enregistrements de",
        "Layout width" => "Largeur du modèle",
        "Don't Save" => "Ne pas enregistrer",
        "Inspector" => "Inspecteur",
        "Selection tool" => "Outil de sélection",
        "Text tool" => "Outil Texte",
        "Field tool" => "Outil Champ",
        "Rectangle tool" => "Outil Rectangle",
        "Line tool" => "Outil Trait",
        "Button tool" => "Outil Bouton",
        "Undo" => "Annuler",
        "Redo" => "Rétablir",
        "Delete" => "Supprimer",
        "Select All" => "Tout sélectionner",
        "Position" => "Position",
        "Appearance" => "Apparence",
        "Data" => "Données",
        "Name" => "Nom",
        "Left" => "Gauche",
        "Top" => "Haut",
        "Right" => "Droite",
        "Bottom" => "Bas",
        "Width" => "Largeur",
        "Height" => "Hauteur",
        "No objects selected" => "Aucun objet sélectionné",
        "objects selected" => "objets sélectionnés",
        "Style" => "Style",
        "Default" => "Par défaut",
        "Fill" => "Remplissage",
        "Border" => "Bordure",
        "Text color" => "Couleur du texte",
        "Use the style's value" => "Utiliser la valeur du style",
        "Field" => "Champ",
        "No field selected" => "Aucun champ sélectionné",
        "Control style" => "Style de contrôle",
        "Edit box" => "Zone de modification",
        "Drop-down list" => "Liste déroulante",
        "Pop-up menu" => "Menu local",
        "Checkbox set" => "Ensemble de cases à cocher",
        "Value list" => "Liste de valeurs",
        "<None>" => "<Aucune>",
        "Specify Field" => "Indiquer le champ",
        "Create label" => "Créer une étiquette",
        "Button Setup" => "Paramétrage du bouton",
        "Button" => "Bouton",
        "Label" => "Libellé",
        "Go to Layout" => "Aller au modèle",
        "Perform Script" => "Exécuter script",
        "Text" => "Texte",
        "Layout" => "Modèle",
        "Header" => "En-tête",
        "Leading Grand Summary" => "Récapitulatif général en tête",
        "Sub-summary" => "Sous-récapitulatif",
        "Body" => "Corps",
        "Trailing Grand Summary" => "Récapitulatif général en fin",
        "Footer" => "Pied de page",
        "Title Header" => "En-tête de titre",
        "Title Footer" => "Pied de page de titre",
        "Top Navigation" => "Navigation supérieure",
        "Bottom Navigation" => "Navigation inférieure",
        "Part" => "Partie",
        "Trailing Sub-summary" => "Sous-récapitulatif en fin",
        "BaseCraft layout objects" => "Objets de modèle BaseCraft",
        "Oval tool" => "Outil Ovale",
        "Portal tool" => "Outil Tableau externe",
        "Tab control tool" => "Outil Contrôle onglet",
        "Rulers" => "Règles",
        "Grid" => "Grille",
        "Snap to grid" => "Aligner sur la grille",
        "Field…" => "Rubrique…",
        "Portal…" => "Tableau externe…",
        "Tab Control…" => "Contrôle onglet…",
        "Button…" => "Bouton…",
        "Line" => "Trait",
        "Rectangle" => "Rectangle",
        "Oval" => "Ovale",
        "Part…" => "Partie…",
        "Layouts" => "Modèles",
        "Arrange" => "Organiser",
        "Cut" => "Couper",
        "Copy" => "Copie",
        "Paste" => "Coller",
        "Bring to Front" => "Mettre au premier plan",
        "Bring Forward" => "Avancer",
        "Send Backward" => "Reculer",
        "Send to Back" => "Mettre à l'arrière-plan",
        "Align" => "Aligner",
        "Left Edges" => "Bords gauches",
        "Centers (horizontally)" => "Centres (horizontalement)",
        "Right Edges" => "Bords droits",
        "Top Edges" => "Bords supérieurs",
        "Centers (vertically)" => "Centres (verticalement)",
        "Bottom Edges" => "Bords inférieurs",
        "Distribute" => "Espacer",
        "Horizontally" => "Horizontalement",
        "Vertically" => "Verticalement",
        "Resize To" => "Redimensionner",
        "Smallest Width" => "Largeur minimale",
        "Smallest Height" => "Hauteur minimale",
        "Smallest Width and Height" => "Largeur et hauteur minimales",
        "Largest Width" => "Largeur maximale",
        "Largest Height" => "Hauteur maximale",
        "Largest Width and Height" => "Largeur et hauteur maximales",
        "Group" => "Associer",
        "Ungroup" => "Dissocier",
        "Lock" => "Verrouiller",
        "Unlock" => "Déverrouiller",
        "New Layout/Report…" => "Nouveau modèle/rapport…",
        "Duplicate Layout" => "Dupliquer le modèle",
        "Delete Layout…" => "Supprimer le modèle…",
        "Styles" => "Styles",
        "Sorted by" => "Trié par",
        "Delete Part" => "Supprimer la partie",
        "Autosizing" => "Dimensionnement automatique",
        "Sliding & visibility" => "Glissement et visibilité",
        "Slide left" => "Glisser vers la gauche",
        "Slide up" => "Glisser vers le haut",
        "Hide when printing" => "Masquer à l'impression",
        "Theme" => "Thème",
        "Clear local changes" => "Effacer les modifications locales",
        "None" => "Aucun",
        "Solid" => "Continu",
        "Dashed" => "Tirets",
        "Dotted" => "Pointillés",
        "Double" => "Double",
        "Radius" => "Rayon",
        "Font" => "Police",
        "Size" => "Taille",
        "Bold" => "Gras",
        "Italic" => "Italique",
        "Underline" => "Souligné",
        "Strikethrough" => "Barré",
        "Color" => "Couleur",
        "Align Left" => "Aligner à gauche",
        "Center" => "Centrer",
        "Align Right" => "Aligner à droite",
        "Justify" => "Justifier",
        "Radio button set" => "Ensemble de boutons radio",
        "Drop-down calendar" => "Calendrier déroulant",
        "Data formatting" => "Formatage des données",
        "As entered (formats are kept, not edited yet)" => {
            "Tel que saisi (les formats sont conservés, pas encore modifiables)"
        }
        "Show related records from" => "Afficher les enregistrements liés de",
        "Initial row" => "Rangée initiale",
        "Number of rows" => "Nombre de rangées",
        "<Select>" => "<Sélectionner>",
        "Fields in the portal" => "Rubriques du tableau externe",
        "Portal Setup" => "Configuration du tableau externe",
        "Tab Control Setup" => "Configuration du contrôle onglet",
        "Tabs (one per line)" => "Onglets (un par ligne)",
        "Part Definition" => "Définition de la partie",
        "When sorted by" => "Lors du tri par",
        "Tab 1" => "Onglet 1",
        "Tab 2" => "Onglet 2",
        "A file needs at least one layout." => "Un fichier doit avoir au moins un modèle.",
        "Script Workspace" => "Espace de travail de script",
        "Script Workspace…" => "Espace de travail de script…",
        "New Script" => "Nouveau script",
        "Rename" => "Renommer",
        "Delete Script" => "Supprimer le script",
        "Run" => "Exécuter",
        "Steps" => "Actions",
        "Search" => "Rechercher",
        "Search functions" => "Rechercher une fonction",
        "Control" => "Contrôle",
        "Navigation" => "Navigation",
        "Editing" => "Edition",
        "Fields" => "Rubriques",
        "Found Sets" => "Enr. trouvés",
        "Windows" => "Fenêtres",
        "Files" => "Fichiers",
        "Accounts" => "Comptes",
        "Miscellaneous" => "Divers",
        "Other" => "Autres",
        "Add criterion" => "Ajouter un critère",
        "Add sort field" => "Ajouter une rubrique de tri",
        "Add steps from the list on the right" => "Ajoutez des actions depuis la liste de droite",
        "By Calculation" => "Par calcul",
        "Calculation:" => "Calcul :",
        "Commit data" => "Valider les données",
        "Continue" => "Continuer",
        "Descending" => "Ordre décroissant",
        "Disable/Enable" => "Désactiver/Activer",
        "Duplicate" => "Dupliquer",
        "Duration:" => "Durée :",
        "Editing this step's options is not supported yet" => {
            "La modification des options de cette action n'est pas encore prise en charge"
        }
        "Exit after last" => "Sortie après dernière",
        "Field:" => "Rubrique :",
        "First" => "Premier",
        "Last" => "Dernier",
        "Previous" => "Précédent",
        "Next" => "Suivant",
        "Flush cached external data" => "Vider les données externes mises en cache",
        "Flush cached join results" => "Vider résultats de jointure en mémoire cache",
        "Folder" => "Dossier",
        "For duration (seconds)" => "Pendant une durée (secondes)",
        "Input field:" => "Rubrique de saisie :",
        "Label:" => "Libellé :",
        "Layout Name by calculation" => "Nom du modèle par calcul",
        "Layout Number by calculation" => "Numéro du modèle par calcul",
        "original layout" => "modèle d'origine",
        "Message:" => "Message :",
        "Move Up" => "Monter",
        "Move Down" => "Descendre",
        "Name:" => "Nom :",
        "No script selected" => "Aucun script sélectionné",
        "Object Name:" => "Nom d'objet :",
        "On" => "Activé",
        "Override ESS locking conflicts" => "Forcer la validation",
        "Parameter:" => "Paramètre :",
        "Pause" => "Pause",
        "Repetition:" => "Répétition :",
        "Result:" => "Résultat :",
        "Script paused" => "Script en pause",
        "Script:" => "Script :",
        "Select a step to edit its options" => "Sélectionnez une action pour modifier ses options",
        "Select entire contents" => "Sélectionner tout le contenu",
        "Select/perform" => "Sélectionner/exécuter",
        "Skip data entry validation" => "Ignorer la validation lors de la saisie des données",
        "Specify Calculation" => "Indiquer le calcul",
        "Specify find requests" => "Indiquer les requêtes",
        "Specify sort order" => "Indiquer l'ordre de tri",
        "This step has no options" => "Cette action n'a pas d'options",
        "Title:" => "Titre :",
        "Value:" => "Valeur :",
        "Variable names start with $ or $$" => "Les noms de variables commencent par $ ou $$",
        "With dialog" => "Avec boîte de dialogue",
        "or variable:" => "ou variable :",
        other => other,
    }
}

/// FileMaker's question when leaving a modified layout.
pub fn save_layout_question(name: &str) -> String {
    if french() {
        format!("Enregistrer les modifications apportées au modèle « {name} » ?")
    } else {
        format!("Save changes to layout \u{201C}{name}\u{201D}?")
    }
}

/// Layouts › Delete Layout's question.
pub fn delete_layout_question(name: &str) -> String {
    if french() {
        format!("Supprimer définitivement le modèle « {name} » ?")
    } else {
        format!("Permanently delete the layout \u{201C}{name}\u{201D}?")
    }
}
