# colle-content

- Programme qui télécharge la page des programmes de cikkes sur le site du
professeur de maths, télécharge les derniers nouveaux PDF, détecte les
exercices CCINP et utilise
[ccinp-extractor](https://github.com/greg904/ccinp-extractor) pour les ajouter
au PDF afin de former un PDF qui peut permettre de réviser plus rapidement. 
- Il faut configurer `cron` ou `systemd` pour lancer automatiquement ce
programme et faire pointer un serveur web vers le dossier dans lequel le
programme met les PDF, afin de l'utiliser avec le lien "programme" pour les
colles MP1 sur [carnot-colles](https://github.com/greg904/carnot-colles).
