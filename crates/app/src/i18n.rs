//! UI strings. English keys; French when `OPENMAKER_LANG` or `LANG` starts with "fr".

use std::sync::OnceLock;

fn french() -> bool {
    static FR: OnceLock<bool> = OnceLock::new();
    *FR.get_or_init(|| {
        ["OPENMAKER_LANG", "LANG", "LC_ALL"]
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
        "Unsaved changes (in memory)" => "Modifications non enregistrées (en mémoire)",
        "or drop a .fmp12 file here" => "ou déposez un fichier .fmp12 ici",
        other => other,
    }
}
