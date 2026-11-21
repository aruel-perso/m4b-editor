# M4B Editor

Français · [English](README.en.md)

Éditeur de livres audio de bureau, avec interface FR/EN et CLI partageant le
même cœur Rust. Version de développement : **0.4.1**.

[Téléchargements officiels](https://github.com/aruel-perso/m4b-editor/releases)

Utilisez le MSI Windows x86_64 ou le DMG macOS Apple Silicon ARM64 correspondant
à votre système. Conservez les archives de sources et les empreintes associées.
Sous Windows, l'application et ses outils multimédias sont installés ensemble ;
le ZIP portable doit être extrait entièrement. Sous Mac, copiez **M4B Editor.app**
dans Applications. Les outils FFmpeg/ffprobe sont inclus ; aucune modification du
PATH n'est requise. La signature éditeur et la notarisation ne sont pas incluses.

Créez ou ouvrez un projet `.m4bproj`, ajoutez les pistes, réglez l'ordre, les
chapitres, les métadonnées et la couverture, puis exportez le livre. Les anciens
projets `.json` restent des formats de compatibilité ; gardez leurs médias.
Le projet et le livre exporté sont deux fichiers distincts. Le CLI s'appelle
`m4b-editor-cli` ; consultez `m4b-editor-cli --help`.

Les exports AAC/M4B, MP3 et FLAC utilisent les encodeurs inclus. L’import ZIP
et RAR fonctionne hors ligne. Consultez le [guide des journaux de diagnostic](docs/diagnostics.md)
en cas de problème.

Les mises à jour exigent une signature Ed25519 et la vérification du contenu.
La migration depuis 0.3 utilise un dépôt de compatibilité et quelques anciens
noms techniques documentés dans [la procédure de livraison](docs/release.md).

[Développement](docs/development.md) · [Tests](docs/testing.md) ·
[Feuille de route](docs/roadmap.md) · [Licence GPL-3.0](LICENSE)

Les distributions doivent conserver les licences des dépendances et les sources
correspondantes. Pas de contournement DRM ni d'édition avancée de forme d'onde.

L’import audio accepte les archives ZIP et RAR. Le champ de mot de passe de l’application permet d’ouvrir les archives chiffrées prises en charge.
