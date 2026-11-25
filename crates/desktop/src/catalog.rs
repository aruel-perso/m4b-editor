//! Central French/English application catalog. Missing keys are programming errors.
pub const CATALOG: &[(&str, &str)] = &[
    ("Move track up", "Monter la piste"),
    ("Move track down", "Descendre la piste"),
    ("Preparing export…", "Préparation de l’export…"),
    ("Checking source files…", "Vérification des fichiers source…"),
    ("Preparing audio tracks…", "Préparation des pistes audio…"),
    ("Encoding audio…", "Encodage audio…"),
    ("Checking output…", "Contrôle du fichier de sortie…"),
    ("Verifying audio…", "Vérification de l’audio…"),
    ("Saving output…", "Enregistrement du fichier…"),
    ("Cancellation requested…", "Annulation demandée…"),
    ("Inspecting input files…", "Inspection des fichiers…"),
    ("Extracting or copying audio…", "Extraction ou copie de l’audio…"),
    ("Analyzing audio…", "Analyse de l’audio…"),
    ("Comparing embedded covers…", "Comparaison des couvertures intégrées…"),
    ("Export diagnostic log…", "Exporter le journal de diagnostic…"),
    ("Open logs folder", "Ouvrir le dossier des journaux"),
    ("Diagnostic log exported", "Journal de diagnostic exporté"),
    ("Export local job activity and error categories. You choose whether to share the file.", "Exportez l’activité locale et les catégories d’erreurs. Vous décidez si vous partagez le fichier."),
    ("Open requested project?", "Ouvrir le projet demandé ?"),

    ("Preparing track", "Préparation de la piste"),
    ("Saved", "Enregistré"),
    (
        "Lossless AAC remux and chapters",
        "Remux AAC sans perte et chapitres",
    ),
    ("Encoding AAC and chapters", "Encodage AAC et chapitres"),
    ("Encoding MP3 and chapters", "Encodage MP3 et chapitres"),
    ("Encoding FLAC and chapters", "Encodage FLAC et chapitres"),
    (
        "Chapter timeline aligned to decoded audio",
        "Chronologie des chapitres alignée sur l’audio décodé",
    ),
    ("source headers", "en-têtes source"),
    ("Operation cancelled", "Opération annulée"),
    (
        "Operation timed out; retry or reduce the job size.",
        "Délai dépassé ; réessayez ou réduisez la taille de l’opération.",
    ),
    (
        "Insufficient disk space; choose another destination or free space.",
        "Espace disque insuffisant ; choisissez une autre destination ou libérez de l’espace.",
    ),
    (
        "Cannot open archive; check its password and format.",
        "Impossible d’ouvrir l’archive ; vérifiez son mot de passe et son format.",
    ),
    (
        "Review chapter positions before changing tracks or exporting.",
        "Vérifiez les positions des chapitres avant de modifier les pistes ou d’exporter.",
    ),
    (
        "Cannot select this audio stream; inspect the available streams.",
        "Impossible de sélectionner ce flux audio ; examinez les flux disponibles.",
    ),
    (
        "Destination exists; choose a new destination.",
        "La destination existe ; choisissez une nouvelle destination.",
    ),
    (
        "The selected encoder is unavailable; check the media backend.",
        "L’encodeur sélectionné est indisponible ; vérifiez le moteur audio.",
    ),
    (
        "Operation failed; open the log for technical details.",
        "Échec de l’opération ; ouvrez le journal pour les détails techniques.",
    ),
    ("Copyright", "Droits d’auteur"),
    ("Cover unavailable", "Couverture indisponible"),
    ("Playback details", "Détails de lecture"),
    (
        "FLAC uses lossless compression; bitrate is automatic.",
        "FLAC utilise une compression sans perte ; le débit est automatique.",
    ),
    ("Project copied", "Projet copié"),
    ("Inspect audio streams", "Examiner les flux audio"),
    ("Absolute audio stream index", "Index absolu du flux audio"),
    ("Add chapter", "Ajouter chapitre"),
    ("Apply selected stream", "Appliquer le flux sélectionné"),
    (
        "Archive password (not saved)",
        "Mot de passe archive (non conservé)",
    ),
    ("Audio", "Audio"),
    ("Audio export", "Export audio"),
    (
        "Audio is copied without re-encoding; bitrate is preserved.",
        "Audio copié sans réencodage ; débit conservé.",
    ),
    ("Author", "Auteur"),
    ("Autosave", "Sauvegarde automatique"),
    ("Bitrate (kb/s)", "Débit (kb/s)"),
    ("Book language (ISO 639-2)", "Langue du livre (ISO 639-2)"),
    ("Book metadata", "Métadonnées du livre"),
    ("Cancel", "Annuler"),
    ("Cancel operation", "Annuler l’opération"),
    ("Chapter", "Chapitre"),
    ("Chapters", "Chapitres"),
    (
        "Chapters — start in milliseconds",
        "Chapitres — début en millisecondes",
    ),
    ("Check for updates…", "Rechercher des mises à jour…"),
    ("Check encoder", "Vérifier encodeur"),
    ("Choose cover…", "Choisir couverture…"),
    ("Close", "Fermer"),
    ("Close project", "Fermer projet"),
    ("Copy portable project…", "Copier le projet portable…"),
    (
        "Create a named project first, then import",
        "Créez un projet nommé, puis importez",
    ),
    ("Create project", "Création du projet"),
    ("Create project…", "Créer un projet…"),
    ("Create…", "Créer…"),
    ("Dark", "Sombre"),
    ("Description", "Description"),
    ("Dismiss", "Ignorer"),
    ("Downloading", "Téléchargement…"),
    ("Encoded excerpt (10 s)", "Extrait encodé (10 s)"),
    ("Encoded transition (6 s)", "Transition encodée (6 s)"),
    ("Encoder available", "Encodeur disponible"),
    ("Encoder unavailable", "Encodeur indisponible"),
    ("Export", "Export"),
    ("Export audio…", "Exporter l’audio…"),
    ("Export complete", "Export terminé"),
    (
        "GitHub updates are checked automatically at startup.",
        "Les mises à jour GitHub sont vérifiées au démarrage.",
    ),
    ("Help", "Aide"),
    ("Image", "Image"),
    ("Import audio / archives…", "Importer audio / archives…"),
    ("Import…", "Importer…"),
    ("Include prereleases", "Inclure les préversions"),
    ("Language", "Langue"),
    ("Later", "Plus tard"),
    ("Light", "Clair"),
    ("Loaded", "Chargé"),
    ("Logs / errors", "Journal / erreurs"),
    (
        "Lossless remux for single AAC M4B/M4A",
        "Remux sans perte AAC",
    ),
    ("M4B Editor project", "Projet M4B Editor"),
    ("Metadata", "Métadonnées"),
    ("Narrator", "Narrateur"),
    ("New project", "Nouveau projet"),
    ("New…", "Nouveau…"),
    ("No cover", "Sans couverture"),
    ("One per track", "Un par piste"),
    ("Open project…", "Ouvrir un projet…"),
    ("Open…", "Ouvrir…"),
    ("Operation details", "Détails de l’opération"),
    ("Overwrite project", "Remplacer le projet"),
    ("Pause", "Pause"),
    ("Play", "Écouter"),
    ("Pre-listening", "Pré-écoute"),
    (
        "Preparing verified installer",
        "Préparation de la mise à jour vérifiée…",
    ),
    ("Project", "Projet"),
    ("Project / M4B", "Projet / M4B"),
    ("Project name", "Nom du projet"),
    ("Project saved", "Projet enregistré"),
    ("Publisher", "Éditeur"),
    ("Recent projects", "Projets récents"),
    ("Remove from recents", "Retirer des projets récents"),
    ("Remove from recent projects?", "Retirer des projets récents ?"),
    ("The project and its files will be kept.", "Le projet et ses fichiers seront conservés."),
    ("Continue to save…", "Continuer vers l’enregistrement…"),
    ("Replace exported audio?", "Remplacer le fichier audio exporté ?"),
    ("The filename extension must match the export format.", "L’extension du fichier doit correspondre au format d’export."),
    ("Choose a readable MPEG-4 audiobook (.m4b).", "Choisissez un livre audio MPEG-4 lisible (.m4b)."),
    ("Recover unsaved project", "Récupérer un projet non enregistré"),
    (
        "An autosave contains unsaved project changes that you can restore.",
        "Une sauvegarde automatique contient des modifications non enregistrées que vous pouvez restaurer.",
    ),
    (
        "This autosave could not be read.",
        "Impossible de lire cette sauvegarde automatique.",
    ),
    ("Autosave date", "Date de sauvegarde automatique"),
    ("Remove cover", "Retirer couverture"),
    ("Replace", "Remplacer"),
    ("Restore", "Restaurer"),
    ("Resume", "Reprendre"),
    ("Save", "Enregistrer"),
    ("Save as…", "Enregistrer sous…"),
    (
        "Save changes before continuing?",
        "Enregistrer les modifications ?",
    ),
    ("Save error", "Erreur d’enregistrement"),
    ("Settings", "Réglages"),
    ("Sidebar width", "Largeur du panneau latéral"),
    ("Stable", "Stable"),
    ("Stop", "Arrêter"),
    ("System", "Système"),
    ("Use project theme", "Utiliser le thème du projet"),
    ("Theme", "Thème"),
    ("Title", "Titre"),
    ("Tracks", "Pistes"),
    (
        "Tracks (custom chapters preserved)",
        "Pistes (chapitres personnalisés conservés)",
    ),
    ("Unsaved project", "Projet modifié"),
    (
        "Project changed on disk; use Save As or reload to preserve the newer version.",
        "Le projet a changé sur le disque ; utilisez Enregistrer sous ou rechargez-le pour préserver la version la plus récente.",
    ),
    ("Update", "Mettre à jour"),
    ("Update available", "Mise à jour disponible"),
    ("Validation", "Validation"),
    ("Verified update", "Mise à jour vérifiée"),
    ("Volume", "Volume"),
    (
        "Waiting for background operation. Close again when finished.",
        "Opération en cours. Fermez à nouveau après sa fin.",
    ),
    (
        "Waiting for media job; export cancellation requested. Close again when finished.",
        "Annulation demandée. Fermez à nouveau après la fin de l’opération.",
    ),
    ("Welcome to M4B Editor", "Bienvenue dans M4B Editor"),
    ("Elapsed / estimated total", "Écoulé / total estimé"),
    ("Elapsed", "Écoulé"),
    ("Unknown", "Inconnu"),
    ("Open release downloads…", "Ouvrir les téléchargements…"),
];
pub fn text(fr: bool, key: &str) -> &'static str {
    let &(en, french) = CATALOG
        .iter()
        .find(|(en, _)| *en == key)
        .unwrap_or_else(|| panic!("Missing translation key: {key}"));
    if fr {
        french
    } else {
        en
    }
}
/// Localize application-generated progress while retaining raw tool diagnostics.
pub fn message(fr: bool, source: &str) -> String {
    if let Some(&(en, _)) = CATALOG
        .iter()
        .find(|(en, french)| *en == source || *french == source)
    {
        return text(fr, en).into();
    }
    for prefix in [
        "Preparing track",
        "Saved",
        "Project copied",
        "Chapter timeline aligned to decoded audio",
    ] {
        if let Some(rest) = source
            .strip_prefix(prefix)
            .filter(|rest| rest.starts_with([' ', ':']))
        {
            let suffix = if prefix == "Chapter timeline aligned to decoded audio" {
                rest.replace("source headers", text(fr, "source headers"))
            } else {
                rest.to_owned()
            };
            return format!("{}{}", text(fr, prefix), suffix);
        }
    }
    source.into()
}
/// All error surfaces use a localized, actionable summary. Detailed backend
/// diagnostics remain in the expandable log rather than falling back to English.
pub fn error(fr: bool, diagnostic: &str) -> &'static str {
    let lower = diagnostic.to_lowercase();
    let key = if lower.contains("cancel") {
        "Operation cancelled"
    } else if lower.contains("deadline") || lower.contains("timed out") {
        "Operation timed out; retry or reduce the job size."
    } else if lower.contains("space") || lower.contains("disk full") {
        "Insufficient disk space; choose another destination or free space."
    } else if lower.contains("password") || lower.contains("encrypt") {
        "Cannot open archive; check its password and format."
    } else if lower.contains("chapter") || lower.contains("padding") {
        "Review chapter positions before changing tracks or exporting."
    } else if lower.contains("stream") {
        "Cannot select this audio stream; inspect the available streams."
    } else if lower.contains("project changed") {
        "Project changed on disk; use Save As or reload to preserve the newer version."
    } else if lower.contains("exists")
        || lower.contains("overwrite")
        || lower.contains("no-overwrite")
    {
        "Destination exists; choose a new destination."
    } else if lower.contains("encoder") {
        "The selected encoder is unavailable; check the media backend."
    } else {
        "Operation failed; open the log for technical details."
    };
    text(fr, key)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_literal_ui_lookup_has_both_languages() {
        let source = include_str!("main.rs");
        for prefix in [
            "self.text(\"",
            "catalog::text(fr, \"",
            "catalog::text(fr,\"",
        ] {
            for part in source.split(prefix).skip(1) {
                let key = part.split('"').next().unwrap();
                assert!(
                    CATALOG.iter().any(|(en, _)| *en == key),
                    "Untranslated UI key: {key}"
                );
            }
        }
    }
    #[test]
    fn app_progress_and_error_summaries_are_localized() {
        assert_eq!(
            message(true, "Preparing track 2: book.wav"),
            "Préparation de la piste 2: book.wav"
        );
        assert_eq!(
            message(true, "Saved /tmp/book.m4b"),
            "Enregistré /tmp/book.m4b"
        );
        assert_eq!(
            message(
                true,
                "Chapter timeline aligned to decoded audio: 42 ms (source headers: 43 ms)"
            ),
            "Chronologie des chapitres alignée sur l’audio décodé: 42 ms (en-têtes source: 43 ms)"
        );
        assert_eq!(
            message(true, "Lossless AAC remux and chapters"),
            "Remux AAC sans perte et chapitres"
        );
        assert_eq!(
            message(true, "Encoding FLAC and chapters"),
            "Encodage FLAC et chapitres"
        );
        assert_eq!(
            message(false, "Encoding MP3 and chapters"),
            "Encoding MP3 and chapters"
        );
        assert_eq!(
            error(true, "Project changed since it was loaded; use Save As or reload"),
            "Le projet a changé sur le disque ; utilisez Enregistrer sous ou rechargez-le pour préserver la version la plus récente."
        );
        assert_eq!(
            error(true, "No space left on device"),
            "Espace disque insuffisant ; choisissez une autre destination ou libérez de l’espace."
        );
        assert_eq!(
            message(true, "[aac] raw diagnostic"),
            "[aac] raw diagnostic"
        );
    }
    #[test]
    fn catalog_has_complete_unique_language_pairs() {
        let mut keys = std::collections::HashSet::new();
        for &(en, fr) in CATALOG {
            assert!(!en.is_empty() && !fr.is_empty());
            assert!(keys.insert(en));
            assert_eq!(text(false, en), en);
            assert_eq!(text(true, en), fr);
        }
    }
}
