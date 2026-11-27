//! UI strings. English keys; French when `BASECRAFT_LANG` or `LANG` starts
//! with "fr" (desktop) or when the browser language is French (web).

use std::sync::OnceLock;

pub fn french() -> bool {
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
        "Help" => "Aide",
        "Open…" => "Ouvrir…",
        "Close" => "Fermer",
        "Exit" => "Quitter",
        "Not implemented yet" => "Pas encore implémenté",
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
        "Accept anyway" => "Accepter quand même",
        "Save a Copy As…" => "Enregistrer une copie sous…",
        "or drop a .fmp12 file here" => "ou déposez un fichier .fmp12 ici",
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
        "New layouts are kept for this session only: writing new layouts to the file is not supported yet." => {
            "Les nouveaux modèles ne sont conservés que pendant cette session : leur écriture dans le fichier n'est pas encore prise en charge."
        }
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
