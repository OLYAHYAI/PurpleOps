# Prototype go/no-go : PurpleOps en Slint (lot L2)

Décision D3 du 09/11/2026 : avant de réécrire l'interface de PurpleOps en Rust/Slint,
on vérifie sur le Pixel 7a les six points où Slint sur Android peut casser. Le
prototype est **isolé** :
- projet Gradle à part ;
- identifiant `com.jbr.purpleops.proto` ;
- APK fourni uniquement comme artefact de CI, jamais en Release ;
- aucun secret, aucune action réelle sur le homelab.

## Architecture testée
- `ProtoActivity` (Kotlin) **hérite de `NativeActivity`**. Slint (`backend-android-activity-06`) dessine toute l'activité.
- Les API Android restent en Kotlin : biométrie et Keystore, ouverture d'autres activités, taille de l'APK. Rust les appelle par **JNI sur l'objet activité** (pas de `FindClass` depuis un fil natif). Kotlin rappelle Rust par des méthodes `external`, puis l'interface est mise à jour par `upgrade_in_event_loop`.
- Thème `ui/theme.slint` = système de design D4 (neutres de soc-display, accent violet).
- Navigation en 4 destinations (D5).

## Les six critères (l'onglet Accueil affiche l'état)
| # | Critère | Comment le tester |
|---|---|---|
| 1 | Coquille NativeActivity + Slint construite par la CI | L'appli s'ouvre et affiche l'Accueil |
| 2 | Saisie texte (IME) avec accents | Opérations : taper `éteindre ÉTEINDRE çà où ; nmap -sV 10.10.10.5`, comparer l'écho, puis « Saisie OK/KO » |
| 3 | Biométrie forte + signature Keystore depuis Rust | Énergie : « Signer (empreinte) ». Attendu : « Signature ECDSA P-256 de ~70 octets, clé en StrongBox (Titan M2) » |
| 4 | Activité Kotlin (WebView) puis retour | Plus : « Ouvrir la WebView », puis Retour. Attendu : retour sur Slint, critère à OK |
| 5 | Taille d'APK et démarrage < 1 s | Accueil, ligne 5 (taille réelle + temps jusqu'à la 1re image), puis « Taille/démarrage OK » |
| 6 | Défilement fluide de 200 lignes | Plus : faire défiler vite, puis « Fluide / Saccadé » |

**Go** si 2 et 3 sont OK : ce sont les deux points bloquants identifiés. 4, 5 et 6 dimensionnent l'effort.
**Repli** si 2 ou 3 échoue : Compose, avec les mêmes tokens générés (D4 inchangée).

Points à observer en plus :
- barre d'état et encoche (marge `top-inset` fixe à 28 px dans le prototype) ;
- rotation ;
- retour depuis l'arrière-plan ;
- clavier qui masque le champ (`adjustResize`).

## Construire
- **CI** : workflow `proto-slint` (PR ou branche `proto/**`), artefact `PurpleOps-proto-slint` (APK + `.sha256`).
- **Aperçu sur poste** : la même interface tourne en bureau (`proto::run_desktop()`, backend winit). `cargo check` passe sur l'hôte.

## Installer sur le téléphone
1. Télécharger l'artefact du run, vérifier `sha256sum -c`.
2. Installer `PurpleOps-proto-slint.apk`. Il est signé par la clé debug de la CI : c'est un prototype, il ne remplace pas PurpleOps.
3. À la fin, le désinstaller. Sa clé Keystore de test (`proto.power.v1`) disparaît avec lui.
