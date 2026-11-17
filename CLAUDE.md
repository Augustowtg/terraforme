# Regras do projeto

Toda alteração no terraforme-cli segue este fluxo, nesta ordem.

1. **Issue**: antes de mudar qualquer coisa, abra uma issue no GitHub
   (`gh issue create`) descrevendo o problema ou a mudança.
2. **Branch nova**: a partir da `main` atualizada, numa branch
   `<tipo>/vX.Y.Z-descricao` (ex.: `fix/v0.14.5-aviso-versao-nova`).
3. **Conventional Commits**: as mensagens de commit e o título do PR usam
   `feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:` etc., com a
   versão no fim, e o corpo fecha a issue (`Closes #N`).
4. **Versão**: cada entrega sobe a versão no `Cargo.toml` (e no
   `Cargo.lock`) e ganha uma entrada no `CHANGELOG.md`.
5. **Testes com o mínimo de regras**: teste só o que a mudança pede, sem
   inventar regras. Ex.: num cadastro de usuário, o e-mail tem que existir e
   ser único; não teste se ele é parecido com outros. Ex.: para o aviso de
   versão nova, o teste é a aba Atualização aparecer ao abrir o terra quando
   há versão nova; nada além disso. `cargo test --locked` e
   `cargo fmt --check` passam antes do commit.
6. **Commit, deploy e merge**: commit, push e PR; com o CI verde, o deploy é
   a tag da versão (`git tag vX.Y.Z && git push origin vX.Y.Z`), que roda o
   `release.yml` e publica a release; depois o merge do PR na `main`.
   Sem a tag não há release, e o terra de quem já instalou não avisa que há
   versão nova.
