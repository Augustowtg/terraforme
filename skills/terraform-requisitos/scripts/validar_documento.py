#!/usr/bin/env python3
"""Valida um documento de requisitos gerado pela skill terraform-requisitos.

Uso: python3 validar_documento.py <documento.md>
Sai com código 1 se houver algum ERRO.
"""
import re
import sys
import unicodedata
from pathlib import Path

SECOES = [
    "controle de versoes", "visao geral", "contextos delimitados", "linguagem ubiqua",
    "modelo de dominio", "requisitos funcionais", "regras de negocio",
    "requisitos nao funcionais", "integracoes externas", "lacunas e duvidas",
    "rastreabilidade",
]

ID_DEF = re.compile(r"^###\s+(R[FN]-\d{3})\b")
ID_REF = re.compile(r"\b(R[FN]-\d{3})\b")
CRASE = re.compile(r"`([^`]*)`")
TIPO = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")
EXT_CODIGO = (
    r"ts|tsx|js|jsx|mjs|py|java|kt|kts|go|rb|cs|php|rs|scala|swift|sql|"
    r"c|cc|cpp|h|hpp|vue|svelte|dart|ex|exs|clj|tf|yaml|yml|json"
)
ARQUIVO_CODIGO = re.compile(rf"[\w./-]+\.(?:{EXT_CODIGO})(?::\d+)?\b", re.IGNORECASE)
PASTA_CODIGO = re.compile(r"\b(?:src|lib|app|internal|pkg|test|tests)/")
IDENT = re.compile(
    r"\b("
    r"[a-z]+[A-Z][A-Za-z0-9]*"          # camelCase
    r"|[A-Z][a-z0-9]+[A-Z][A-Za-z0-9]*"  # PascalCase composto
    r"|[a-z0-9]+_[a-z0-9_]+"             # snake_case
    r"|\w+\(\)"                          # chamada()
    r")"
)
PAR_TIPO = re.compile(r"([^;|]+?)\s+é\s+`([^`]+)`")


def normalizar(texto):
    sem_acento = unicodedata.normalize("NFKD", texto).encode("ascii", "ignore").decode()
    return re.sub(r"^[\d.\s]+", "", sem_acento.lower()).strip()


def colunas(linha):
    return [c.strip() for c in linha.strip().strip("|").split("|")]


def eh_separador(linha):
    return bool(re.fullmatch(r"\|?[\s:|-]+\|?", linha.strip()))


def validar(caminho):
    erros, avisos = [], []
    linhas = caminho.read_text(encoding="utf-8").splitlines()

    secao = None
    secoes_vistas = set()
    em_bloco = False
    definidos, referencias, blocos = {}, [], []
    bloco_atual = None
    glossario = {}  # termo normalizado -> tipo ("" quando "—")
    pares_rastreio = []
    versoes = []
    versao_doc = None
    cabecalho_tabela = set()

    for n, linha in enumerate(linhas, 1):
        if linha.strip().startswith("```"):
            em_bloco = not em_bloco
            continue
        if em_bloco:
            continue

        # Arquivos e pastas de código nunca podem aparecer.
        for m in ARQUIVO_CODIGO.finditer(linha):
            erros.append(f"linha {n}: referência a arquivo de código não é permitida: {m.group(0)}")
        if PASTA_CODIGO.search(linha):
            erros.append(f"linha {n}: referência a pasta de código não é permitida")

        if linha.startswith("## "):
            secao = normalizar(linha[3:])
            for s in SECOES:
                if secao.startswith(s):
                    secoes_vistas.add(s)
                    secao = s
            if bloco_atual:
                blocos.append(bloco_atual)
                bloco_atual = None
            cabecalho_tabela = set()
            continue

        m = ID_DEF.match(linha)
        if m:
            if bloco_atual:
                blocos.append(bloco_atual)
            rid = m.group(1)
            if rid in definidos:
                erros.append(f"linha {n}: {rid} definido de novo (primeira vez na linha {definidos[rid]})")
            definidos[rid] = n
            bloco_atual = [rid, n, ""]
            continue
        if bloco_atual:
            bloco_atual[2] += linha + "\n"

        if linha.lstrip().startswith(">"):
            continue

        eh_linha_tabela = linha.lstrip().startswith("|")
        if eh_linha_tabela and eh_separador(linha):
            continue
        # Primeira linha de cada tabela é o cabeçalho.
        primeira_da_tabela = eh_linha_tabela and secao not in cabecalho_tabela
        if primeira_da_tabela:
            cabecalho_tabela.add(secao)
            continue

        if secao == "controle de versoes" and eh_linha_tabela:
            v = colunas(linha)[0]
            if not SEMVER.match(v):
                erros.append(f"linha {n}: versão '{v}' fora do formato MAIOR.MENOR.CORREÇÃO")
            else:
                versoes.append((tuple(map(int, v.split("."))), v, n))
            continue

        if secao == "visao geral":
            mv = re.search(r"vers[aã]o do documento:\**\s*([^\s]+)", linha, re.IGNORECASE)
            if mv:
                versao_doc = (mv.group(1), n)

        if secao == "rastreabilidade":
            if eh_linha_tabela:
                cols = colunas(linha)
                corresp = cols[1] if len(cols) > 1 else ""
                pares = PAR_TIPO.findall(corresp)
                if CRASE.findall(corresp) and len(pares) != len(CRASE.findall(corresp)):
                    erros.append(f"linha {n}: use o formato 'Termo é `Tipo`' em cada correspondência")
                for termo, tipo in pares:
                    pares_rastreio.append((termo.strip(), tipo, n))
            continue

        if secao != "controle de versoes":
            referencias += [(r, n) for r in ID_REF.findall(linha)]

        texto = linha
        if secao == "linguagem ubiqua" and eh_linha_tabela:
            cols = colunas(linha)
            if len(cols) >= 3:
                tipos = CRASE.findall(cols[2])
                if not tipos and cols[2] not in ("—", "-", ""):
                    avisos.append(f"linha {n}: 'Termo no código' deve ser `Tipo` ou '—'")
                for t in tipos:
                    if not TIPO.match(t):
                        erros.append(f"linha {n}: '{t}' não é nome de tipo (sem métodos, atributos ou caminhos)")
                glossario[normalizar(cols[0])] = tipos[0] if tipos else ""
                texto = "|".join(cols[:2] + cols[3:])

        # Referências a outras documentações (arquivos .md, .pdf etc.) podem ficar entre crases.
        crases = [c for c in CRASE.findall(texto) if not re.search(r"\.(md|pdf|docx?|txt|html?)$", c)]
        if crases:
            avisos.append(f"linha {n}: termo de código fora do glossário e da rastreabilidade: {', '.join(crases)}")
        soltos = IDENT.findall(CRASE.sub("", texto))
        if soltos:
            avisos.append(f"linha {n}: parece identificador de código no texto: {', '.join(sorted(set(soltos)))}")

    if bloco_atual:
        blocos.append(bloco_atual)

    for s in SECOES:
        if s not in secoes_vistas:
            erros.append(f"Seção obrigatória ausente: '{s}'")

    if not versoes:
        erros.append("Controle de versões sem nenhuma versão")
    else:
        for (a, _, _), (b, vb, nb) in zip(versoes, versoes[1:]):
            if b <= a:
                erros.append(f"linha {nb}: versão {vb} não é maior que a anterior")
        if not versao_doc:
            erros.append("Visão geral sem 'Versão do documento'")
        elif versao_doc[0] != versoes[-1][1]:
            erros.append(
                f"linha {versao_doc[1]}: 'Versão do documento' ({versao_doc[0]}) difere da "
                f"última versão do controle ({versoes[-1][1]})"
            )

    for rid, n in referencias:
        if rid not in definidos:
            erros.append(f"linha {n}: {rid} citado mas não definido")

    for termo, tipo, n in pares_rastreio:
        if not TIPO.match(tipo):
            erros.append(f"linha {n}: '{tipo}' não é nome de tipo (sem métodos, atributos ou caminhos)")
        chave = normalizar(termo)
        if chave not in glossario:
            avisos.append(f"linha {n}: '{termo}' não está no glossário")
        elif glossario[chave] and glossario[chave] != tipo:
            erros.append(f"linha {n}: '{termo}' é `{glossario[chave]}` no glossário, mas `{tipo}` aqui")

    ids_rastreio = set()
    for linha in linhas:
        if linha.lstrip().startswith("|"):
            ids_rastreio.update(ID_REF.findall(colunas(linha)[0]))
    for rid in definidos:
        if rid not in ids_rastreio:
            avisos.append(f"{rid} sem linha na rastreabilidade")

    for rid, n, texto in blocos:
        t = normalizar(texto.replace("*", ""))
        if rid.startswith("RF") and "criterios de aceite" not in t:
            erros.append(f"{rid} (linha {n}) sem 'Critérios de aceite'")
        if rid.startswith("RF") and not re.search(r"\bdad[oa]\b.*\bquando\b.*\bentao\b", t):
            avisos.append(f"{rid} (linha {n}) sem critério no formato Dado/Quando/Então")

    rf = sum(1 for r in definidos if r.startswith("RF"))
    rn = sum(1 for r in definidos if r.startswith("RN"))
    if rf == 0:
        erros.append("Nenhum requisito funcional (### RF-001 ...) definido")
    return erros, avisos, rf, rn, (versoes[-1][1] if versoes else None)


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    caminho = Path(sys.argv[1]).expanduser()
    if not caminho.is_file():
        print(f"ERRO: arquivo não encontrado: {caminho}")
        sys.exit(2)
    erros, avisos, rf, rn, versao = validar(caminho)
    for e in erros:
        print(f"ERRO: {e}")
    for a in avisos:
        print(f"AVISO: {a}")
    if not erros:
        print(
            f"OK: versão {versao}, {rf} requisitos funcionais e {rn} regras de negócio"
            + (" (com avisos)" if avisos else "")
        )
    sys.exit(1 if erros else 0)


if __name__ == "__main__":
    main()
