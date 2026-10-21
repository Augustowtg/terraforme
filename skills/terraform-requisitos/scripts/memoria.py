#!/usr/bin/env python3
"""Memória do terraform-requisitos: guarda o levantamento em ~/.terraforme/<projeto>/requisitos.json.

O JSON é a fonte da verdade. Cada achado entra por partes (seção + id + campo), e o .md e o PDF
são sempre gerados a partir dele. Só usa a biblioteca padrão; o PDF chama gerar_pdf.py.

Uso:
    memoria.py iniciar  <projeto> --titulo "Loja Online" [--autor A] [--data AAAA-MM-DD]
    memoria.py inserir  <projeto> <secao> --json '{...}'               # devolve o id criado
    memoria.py editar   <projeto> <secao> [<id>] --campo C --valor V [--anexar]
    memoria.py editar   <projeto> <secao> [<id>] --json '{...}'        # mescla só os campos informados
    memoria.py ler      <projeto> [<secao> [<id>]] [--campo C]
    memoria.py buscar   <projeto> <texto> [--secao S] [--json]
    memoria.py remover  <projeto> <secao> <id> [--versao X.Y.Z]
    memoria.py versao   <projeto> --tipo maior|menor|correcao --mudancas "..." [--autor A] [--data D]
    memoria.py verificar <projeto>
    memoria.py gerar    <projeto> [--md caminho.md] [--pdf] [--sem-sumario]
    memoria.py importar <projeto> --arquivo dados.json [--substituir]
    memoria.py caminho  <projeto>

Seções: documento, visao_geral, contextos, glossario, modelo_dominio (dominio),
requisitos_funcionais (rf), regras_negocio (rn), requisitos_nao_funcionais (rnf), integracoes, lacunas.
O esquema completo está em references/esquema-json.md.
A raiz ~/.terraforme (%APPDATA%\\.terraforme no Windows) pode ser trocada pela variável de
ambiente TERRAFORME_HOME.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unicodedata
from datetime import date, datetime
from pathlib import Path

ESQUEMA = "terraforme-requisitos/1"
SCRIPTS = Path(__file__).resolve().parent

# Seções que são listas de itens com id: prefixo do id, campos obrigatórios e valores padrão.
LISTAS = {
    "contextos": ("CTX", ["nome", "responsabilidade"], {"relacoes": ""}),
    "glossario": ("GLO", ["termo", "definicao", "contexto"], {"tipo_codigo": None}),
    "modelo_dominio": ("DOM", ["contexto"], {
        "agregados": [], "entidades": [], "objetos_valor": [], "ciclo_vida": [], "eventos": [],
    }),
    "requisitos_funcionais": ("RF", ["titulo", "descricao", "ator", "contexto", "criterios"], {
        "regras": [], "referencias": None, "rastreabilidade": [], "removido_em": None,
    }),
    "regras_negocio": ("RN", ["titulo", "regra", "contexto", "consequencia"], {
        "referencias": None, "rastreabilidade": [], "removido_em": None,
    }),
    "requisitos_nao_funcionais": ("RNF", ["categoria", "descricao"], {"removido_em": None}),
    "integracoes": ("INT", ["sistema", "finalidade", "direcao", "contexto"], {}),
    "lacunas": ("LAC", ["descricao"], {}),
}
# Seções que são um objeto único (editadas sem id).
OBJETOS = {
    "documento": {"titulo": "", "nota": None},
    "visao_geral": {"proposito": None, "escopo": None, "versao_aplicacao": None,
                    "documentos_relacionados": None},
}
APELIDOS = {
    "rf": "requisitos_funcionais", "rn": "regras_negocio", "rnf": "requisitos_nao_funcionais",
    "dominio": "modelo_dominio", "ctx": "contextos", "glo": "glossario", "int": "integracoes",
    "lac": "lacunas", "visao": "visao_geral", "doc": "documento",
}
REMOCAO_SUAVE = {"requisitos_funcionais", "regras_negocio", "requisitos_nao_funcionais"}
ITENS_DOMINIO = ("agregados", "entidades", "objetos_valor", "eventos")

TIPO = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")
DATA = re.compile(r"^\d{4}-\d{2}-\d{2}$")
PROJETO = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")


class ErroMemoria(Exception):
    pass


# ---------------------------------------------------------------- armazenamento

def raiz():
    """Pasta local do terraforme: ~/.terraforme no Linux, %APPDATA%\\.terraforme no Windows."""
    if os.environ.get("TERRAFORME_HOME"):
        return Path(os.environ["TERRAFORME_HOME"]).expanduser()
    if os.name == "nt" and os.environ.get("APPDATA"):
        return Path(os.environ["APPDATA"]) / ".terraforme"
    return Path("~/.terraforme").expanduser()


def pasta_projeto(projeto):
    if not PROJETO.match(projeto) or ".." in projeto:
        raise ErroMemoria(f"nome de projeto inválido: '{projeto}' (use letras, números, '.', '-' e '_')")
    return raiz() / projeto


def arquivo_json(projeto):
    return pasta_projeto(projeto) / "requisitos.json"


def carregar(projeto):
    caminho = arquivo_json(projeto)
    if not caminho.exists():
        raise ErroMemoria(f"projeto '{projeto}' não existe; rode 'iniciar' primeiro ({caminho})")
    dados = json.loads(caminho.read_text(encoding="utf-8"))
    if dados.get("esquema") != ESQUEMA:
        raise ErroMemoria(f"esquema desconhecido em {caminho}: {dados.get('esquema')}")
    return dados


def salvar(projeto, dados):
    """Grava de forma atômica: escreve num temporário e troca, para o JSON nunca ficar pela metade."""
    caminho = arquivo_json(projeto)
    caminho.parent.mkdir(parents=True, exist_ok=True)
    dados["atualizado_em"] = datetime.now().isoformat(timespec="seconds")
    fd, tmp = tempfile.mkstemp(dir=caminho.parent, prefix=".requisitos-", suffix=".json")
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(dados, f, ensure_ascii=False, indent=2)
        f.write("\n")
    os.replace(tmp, caminho)


def vazio(projeto, titulo):
    return {
        "esquema": ESQUEMA,
        "projeto": projeto,
        "atualizado_em": None,
        "documento": {**OBJETOS["documento"], "titulo": titulo},
        "versoes": [],
        "visao_geral": dict(OBJETOS["visao_geral"]),
        **{secao: [] for secao in LISTAS},
        "contadores": {prefixo: 0 for prefixo, _, _ in LISTAS.values()},
    }


# ---------------------------------------------------------------- validação de itens

def secao_canonica(nome):
    nome = APELIDOS.get(nome.lower(), nome.lower())
    if nome not in LISTAS and nome not in OBJETOS and nome != "versoes":
        validas = ", ".join(list(OBJETOS) + list(LISTAS))
        raise ErroMemoria(f"seção desconhecida: '{nome}'. Seções: {validas}")
    return nome


def campos_permitidos(secao):
    if secao in OBJETOS:
        return set(OBJETOS[secao])
    _, obrigatorios, padroes = LISTAS[secao]
    return {"id", *obrigatorios, *padroes}


def lista_de_textos(valor, campo):
    if not isinstance(valor, list) or not all(isinstance(v, str) and v.strip() for v in valor):
        raise ErroMemoria(f"'{campo}' deve ser uma lista de textos não vazios")


def checar_item(secao, item):
    """Confere tipos e formatos de um item completo da seção."""
    desconhecidos = set(item) - campos_permitidos(secao)
    if desconhecidos:
        raise ErroMemoria(f"campo(s) desconhecido(s) em {secao}: {', '.join(sorted(desconhecidos))}. "
                          f"Permitidos: {', '.join(sorted(campos_permitidos(secao)))}")
    if secao in OBJETOS:
        if secao == "documento" and not str(item.get("titulo") or "").strip():
            raise ErroMemoria("documento.titulo não pode ficar vazio")
        return
    _, obrigatorios, _ = LISTAS[secao]
    for campo in obrigatorios:
        valor = item.get(campo)
        if valor is None or (isinstance(valor, (str, list)) and not valor):
            raise ErroMemoria(f"campo obrigatório ausente em {secao}: '{campo}'")
    if secao == "glossario":
        tipo = item.get("tipo_codigo")
        if tipo is not None and not TIPO.match(str(tipo)):
            raise ErroMemoria(f"tipo_codigo '{tipo}' não é nome de tipo (sem métodos, atributos ou caminhos); "
                              "use null quando não houver tipo")
    if secao == "requisitos_funcionais":
        lista_de_textos(item["criterios"], "criterios")
        if not all(re.fullmatch(r"RN-\d{3}", r) for r in item.get("regras", [])):
            raise ErroMemoria("'regras' deve ser uma lista de ids no formato RN-001")
    if secao in ("requisitos_funcionais", "regras_negocio") and item.get("rastreabilidade"):
        lista_de_textos(item["rastreabilidade"], "rastreabilidade")
    if secao == "modelo_dominio":
        for campo in ITENS_DOMINIO:
            for parte in item.get(campo, []):
                if not isinstance(parte, dict) or set(parte) != {"nome", "descricao"}:
                    raise ErroMemoria(f"cada item de '{campo}' deve ser {{\"nome\": ..., \"descricao\": ...}}")
        if item.get("ciclo_vida"):
            lista_de_textos(item["ciclo_vida"], "ciclo_vida")
    removido = item.get("removido_em")
    if removido is not None and not SEMVER.match(str(removido)):
        raise ErroMemoria(f"removido_em '{removido}' fora do formato MAIOR.MENOR.CORREÇÃO")


def chave_unica(secao, item):
    if secao == "glossario":
        return normalizar(item["termo"])
    if secao == "contextos":
        return normalizar(item["nome"])
    if secao == "modelo_dominio":
        return normalizar(item["contexto"])
    return None


def normalizar(texto):
    sem_acento = unicodedata.normalize("NFKD", str(texto)).encode("ascii", "ignore").decode()
    return sem_acento.lower().strip()


def checar_duplicado(dados, secao, item):
    chave = chave_unica(secao, item)
    if chave is None:
        return
    for outro in dados[secao]:
        if outro["id"] != item["id"] and chave_unica(secao, outro) == chave:
            raise ErroMemoria(f"já existe em {secao} um item com o mesmo nome: {outro['id']}")


def achar(dados, secao, id_):
    for item in dados[secao]:
        if item["id"] == id_:
            return item
    raise ErroMemoria(f"{id_} não existe em {secao}")


def interpretar_valor(texto):
    """Aceita JSON (listas, objetos, null, números); qualquer outra coisa vira texto."""
    try:
        return json.loads(texto)
    except (json.JSONDecodeError, TypeError):
        return texto


def ler_json_arg(texto):
    try:
        valor = json.loads(texto)
    except json.JSONDecodeError as e:
        raise ErroMemoria(f"--json inválido: {e}")
    if not isinstance(valor, dict):
        raise ErroMemoria("--json deve ser um objeto")
    return valor


# ---------------------------------------------------------------- operações

def iniciar(projeto, titulo, autor="—", data=None):
    if arquivo_json(projeto).exists():
        raise ErroMemoria(f"projeto '{projeto}' já existe em {arquivo_json(projeto)}")
    dados = vazio(projeto, titulo)
    dados["versoes"].append({"versao": "1.0.0", "data": data or date.today().isoformat(),
                             "autor": autor, "mudancas": "Versão inicial"})
    salvar(projeto, dados)
    return arquivo_json(projeto)


def inserir(dados, secao, item):
    if secao not in LISTAS:
        raise ErroMemoria(f"'{secao}' não é uma lista; use 'editar' para alterar seus campos")
    prefixo, _, padroes = LISTAS[secao]
    contador = dados["contadores"].get(prefixo, 0)
    id_ = item.get("id")
    if id_ is None:
        contador += 1
        id_ = f"{prefixo}-{contador:03d}"
    else:
        m = re.fullmatch(rf"{prefixo}-(\d{{3}})", str(id_))
        if not m:
            raise ErroMemoria(f"id '{id_}' fora do formato {prefixo}-001")
        if any(i["id"] == id_ for i in dados[secao]):
            raise ErroMemoria(f"{id_} já existe em {secao}; use 'editar'")
        if int(m.group(1)) <= contador:
            raise ErroMemoria(f"{id_} já foi usado antes; ids nunca são reaproveitados")
        contador = int(m.group(1))
    novo = {"id": id_, **{k: (list(v) if isinstance(v, list) else v) for k, v in padroes.items()}, **item}
    novo["id"] = id_
    checar_item(secao, novo)
    checar_duplicado(dados, secao, novo)
    dados[secao].append(novo)
    dados["contadores"][prefixo] = contador
    return id_


def alvo(dados, secao, id_):
    if secao == "versoes":
        raise ErroMemoria("o controle de versões só muda pelo comando 'versao'")
    if secao in OBJETOS:
        if id_:
            raise ErroMemoria(f"'{secao}' não tem itens com id; informe só o campo")
        return dados[secao]
    if not id_:
        raise ErroMemoria(f"informe o id do item de {secao}")
    return achar(dados, secao, id_)


def editar(dados, secao, id_, mudancas, anexar=False):
    """Mescla só os campos informados; com anexar, acrescenta o valor a um campo de lista."""
    atual = alvo(dados, secao, id_)
    if "id" in mudancas:
        raise ErroMemoria("o id não pode ser alterado")
    novo = dict(atual)
    for campo, valor in mudancas.items():
        if anexar:
            if not isinstance(novo.get(campo), list):
                raise ErroMemoria(f"'{campo}' não é uma lista; --anexar só vale para listas")
            novo[campo] = novo[campo] + (valor if isinstance(valor, list) else [valor])
        else:
            novo[campo] = valor
    checar_item(secao, novo)
    if secao in LISTAS:
        checar_duplicado(dados, secao, novo)
    atual.clear()
    atual.update(novo)
    return atual


def ler(dados, secao=None, id_=None, campo=None):
    if secao is None:
        return dados
    valor = dados[secao] if id_ is None and secao not in OBJETOS else alvo(dados, secao, id_)
    if campo is not None:
        if not isinstance(valor, dict) or campo not in valor:
            raise ErroMemoria(f"campo '{campo}' não existe")
        return valor[campo]
    return valor


def buscar(dados, texto, secao=None):
    """Procura o texto (sem diferenciar maiúsculas e acentos) em qualquer campo. Devolve (secao, id, campo, valor)."""
    alvo_txt = normalizar(texto)
    achados = []

    def varrer(sec, id_, caminho, valor):
        if isinstance(valor, dict):
            for k, v in valor.items():
                if k != "id":
                    varrer(sec, id_, f"{caminho}.{k}" if caminho else k, v)
        elif isinstance(valor, list):
            for i, v in enumerate(valor):
                varrer(sec, id_, f"{caminho}[{i}]", v)
        elif isinstance(valor, str) and alvo_txt in normalizar(valor):
            achados.append((sec, id_, caminho, valor))

    secoes = [secao] if secao else list(OBJETOS) + ["versoes"] + list(LISTAS)
    for sec in secoes:
        conteudo = dados[sec]
        if isinstance(conteudo, dict):
            varrer(sec, None, "", conteudo)
        else:
            for i, item in enumerate(conteudo):
                varrer(sec, item.get("id") or item.get("versao") or str(i), "", item)
    return achados


def remover(dados, secao, id_, versao=None):
    """RF, RN e RNF continuam no documento marcados como removidos; o resto sai de vez. O id nunca volta."""
    if secao not in LISTAS:
        raise ErroMemoria(f"'{secao}' não tem itens para remover")
    item = achar(dados, secao, id_)
    if secao in REMOCAO_SUAVE:
        versao = versao or dados["versoes"][-1]["versao"]
        if not SEMVER.match(versao):
            raise ErroMemoria(f"versão '{versao}' fora do formato MAIOR.MENOR.CORREÇÃO")
        item["removido_em"] = versao
        return "marcado"
    dados[secao].remove(item)
    return "apagado"


def nova_versao(dados, tipo, mudancas, autor=None, data=None):
    ultima = dados["versoes"][-1]
    maior, menor, correcao = map(int, ultima["versao"].split("."))
    if tipo == "maior":
        versao = f"{maior + 1}.0.0"
    elif tipo == "menor":
        versao = f"{maior}.{menor + 1}.0"
    elif tipo == "correcao":
        versao = f"{maior}.{menor}.{correcao + 1}"
    else:
        raise ErroMemoria("tipo de versão deve ser maior, menor ou correcao")
    data = data or date.today().isoformat()
    if not DATA.match(data):
        raise ErroMemoria(f"data '{data}' fora do formato AAAA-MM-DD")
    if not mudancas.strip():
        raise ErroMemoria("descreva as mudanças da versão")
    dados["versoes"].append({"versao": versao, "data": data, "autor": autor or ultima["autor"],
                             "mudancas": mudancas})
    return versao


def verificar(dados):
    """Confere as ligações entre seções. Devolve (erros, avisos)."""
    erros, avisos = [], []
    if not dados["versoes"]:
        erros.append("controle de versões vazio")
    rns = {r["id"] for r in dados["regras_negocio"]}
    glossario = {normalizar(g["termo"]): g for g in dados["glossario"]}
    contextos = {normalizar(c["nome"]) for c in dados["contextos"]} | {"compartilhado"}
    if not [r for r in dados["requisitos_funcionais"]]:
        erros.append("nenhum requisito funcional registrado")
    for secao in ("requisitos_funcionais", "regras_negocio"):
        for item in dados[secao]:
            for rn in item.get("regras", []):
                if rn not in rns:
                    erros.append(f"{item['id']} cita {rn}, que não existe")
            if not item["rastreabilidade"]:
                avisos.append(f"{item['id']} sem rastreabilidade")
            for termo in item["rastreabilidade"]:
                g = glossario.get(normalizar(termo))
                if g is None:
                    erros.append(f"{item['id']}: '{termo}' na rastreabilidade não está no glossário")
                elif not g.get("tipo_codigo"):
                    erros.append(f"{item['id']}: '{termo}' não tem tipo no código e não pode ir para a rastreabilidade")
    for secao in ("glossario", "requisitos_funcionais", "regras_negocio", "integracoes", "modelo_dominio"):
        for item in dados[secao]:
            if contextos and normalizar(item["contexto"]) not in contextos:
                avisos.append(f"{item['id']}: contexto '{item['contexto']}' não está em contextos")
    return erros, avisos


# ---------------------------------------------------------------- geração do Markdown

def celula(texto):
    return str(texto).replace("|", "\\|").replace("\n", " ")


def ou(valor, padrao="Não identificado"):
    return valor if valor not in (None, "", []) else padrao


def titulo_item(item):
    sufixo = f" (removido na versão {item['removido_em']})" if item.get("removido_em") else ""
    return f"{item['id']} — {item['titulo']}{sufixo}"


def renderizar(dados):
    doc, visao = dados["documento"], dados["visao_geral"]
    L = [f"# Documento de Requisitos — {doc['titulo']}", ""]
    if doc.get("nota"):
        L += [f"> {linha}".rstrip() for linha in doc["nota"].splitlines()] + [""]

    L += ["## Controle de versões", "", "| Versão | Data | Autor | Mudanças |", "|---|---|---|---|"]
    L += [f"| {v['versao']} | {v['data']} | {celula(v['autor'])} | {celula(v['mudancas'])} |" for v in dados["versoes"]]

    L += ["", "## 1. Visão geral", "",
          f"- **Propósito:** {ou(visao.get('proposito'))}",
          f"- **Escopo:** {ou(visao.get('escopo'))}",
          f"- **Versão do documento:** {dados['versoes'][-1]['versao']}",
          f"- **Versão da aplicação analisada:** {ou(visao.get('versao_aplicacao'))}",
          f"- **Documentos relacionados:** {ou(visao.get('documentos_relacionados'), 'Nenhum')}"]

    L += ["", "## 2. Contextos delimitados", ""]
    if dados["contextos"]:
        L += ["| Contexto | Responsabilidade | Relação com outros contextos |", "|---|---|---|"]
        L += [f"| {celula(c['nome'])} | {celula(c['responsabilidade'])} | {celula(ou(c.get('relacoes'), '—'))} |"
              for c in dados["contextos"]]
    else:
        L.append("Não identificado.")

    L += ["", "## 3. Linguagem ubíqua", ""]
    if dados["glossario"]:
        L += ["| Termo de negócio | Definição | Termo no código | Contexto |", "|---|---|---|---|"]
        for g in dados["glossario"]:
            tipo = f"`{g['tipo_codigo']}`" if g.get("tipo_codigo") else "—"
            L.append(f"| {celula(g['termo'])} | {celula(g['definicao'])} | {tipo} | {celula(g['contexto'])} |")
    else:
        L.append("Não identificado.")

    L += ["", "## 4. Modelo de domínio", ""]
    if not dados["modelo_dominio"]:
        L += ["Não identificado.", ""]
    rotulos = [("agregados", "Agregados"), ("entidades", "Entidades"), ("objetos_valor", "Objetos de valor"),
               ("ciclo_vida", "Ciclo de vida"), ("eventos", "Eventos de domínio")]
    for n, dom in enumerate(dados["modelo_dominio"], 1):
        L += [f"### 4.{n} {dom['contexto']}", ""]
        for campo, rotulo in rotulos:
            L += [f"**{rotulo}**", ""]
            partes = dom.get(campo) or []
            if not partes:
                L.append("- Não identificado.")
            elif campo == "ciclo_vida":
                L += [f"- {p}" for p in partes]
            else:
                L += [f"- **{p['nome']}:** {p['descricao']}" for p in partes]
            L.append("")

    L += ["## 5. Requisitos funcionais", ""]
    if not dados["requisitos_funcionais"]:
        L += ["Não identificado.", ""]
    for rf in dados["requisitos_funcionais"]:
        L += [f"### {titulo_item(rf)}", "",
              f"- **Descrição:** {rf['descricao']}",
              f"- **Ator:** {rf['ator']}",
              f"- **Contexto:** {rf['contexto']}",
              f"- **Regras relacionadas:** {', '.join(rf['regras']) or 'Nenhuma'}",
              "- **Critérios de aceite:**"]
        L += [f"  - {c}" for c in rf["criterios"]]
        if rf.get("referencias"):
            L.append(f"- **Referências:** {rf['referencias']}")
        L.append("")

    L += ["## 6. Regras de negócio", ""]
    if not dados["regras_negocio"]:
        L += ["Não identificado.", ""]
    for rn in dados["regras_negocio"]:
        L += [f"### {titulo_item(rn)}", "",
              f"- **Regra:** {rn['regra']}",
              f"- **Contexto:** {rn['contexto']}",
              f"- **Consequência quando violada:** {rn['consequencia']}"]
        if rn.get("referencias"):
            L.append(f"- **Referências:** {rn['referencias']}")
        L.append("")

    L += ["## 7. Requisitos não funcionais", ""]
    for rnf in dados["requisitos_nao_funcionais"]:
        sufixo = f" (removido na versão {rnf['removido_em']})" if rnf.get("removido_em") else ""
        L.append(f"- **{rnf['id']} — {rnf['categoria']}{sufixo}:** {rnf['descricao']}")
    if not dados["requisitos_nao_funcionais"]:
        L.append("Não identificado.")

    L += ["", "## 8. Integrações externas", ""]
    if dados["integracoes"]:
        L += ["| Sistema externo | Para que é usado | Direção | Contexto |", "|---|---|---|---|"]
        L += [f"| {celula(i['sistema'])} | {celula(i['finalidade'])} | {celula(i['direcao'])} | {celula(i['contexto'])} |"
              for i in dados["integracoes"]]
    else:
        L.append("Não identificado.")

    L += ["", "## 9. Lacunas e dúvidas", ""]
    L += [f"- {lac['descricao']}" for lac in dados["lacunas"]] or ["Nenhuma."]

    L += ["", "## 10. Rastreabilidade", ""]
    tipos = {normalizar(g["termo"]): g for g in dados["glossario"]}
    linhas = []
    for item in dados["requisitos_funcionais"] + dados["regras_negocio"]:
        pares = []
        for termo in item["rastreabilidade"]:
            g = tipos[normalizar(termo)]
            pares.append(f"{g['termo']} é `{g['tipo_codigo']}`")
        if pares:
            linhas.append(f"| {item['id']} | {'; '.join(pares)} |")
    if linhas:
        L += ["| Requisito | Correspondência no código |", "|---|---|"] + linhas
    else:
        L.append("Não identificado.")
    return "\n".join(L) + "\n"


def gerar_pdf(md, sem_sumario=False):
    script = SCRIPTS / "gerar_pdf.py"
    pdf = md.with_name(f"{md.stem}.pdf")
    uv = shutil.which("uv")
    comando = ([uv, "run", "--quiet", str(script)] if uv else [sys.executable, str(script)]) + [str(md), "-o", str(pdf)]
    if sem_sumario:
        comando.append("--sem-sumario")
    r = subprocess.run(comando, capture_output=True, text=True)
    if r.returncode != 0:
        raise ErroMemoria(f"falha ao gerar o PDF:\n{r.stdout}{r.stderr}")
    return pdf


def gerar(projeto, md=None, pdf=False, sem_sumario=False):
    """Gera o .md (e o PDF, se pedido) a partir do JSON. Devolve (md, pdfs, erros_validador, avisos_validador)."""
    dados = carregar(projeto)
    erros, _ = verificar(dados)
    if erros:
        raise ErroMemoria("o JSON tem inconsistências; corrija antes de gerar:\n  - " + "\n  - ".join(erros))
    md = Path(md).expanduser() if md else pasta_projeto(projeto) / "requisitos.md"
    md.parent.mkdir(parents=True, exist_ok=True)
    md.write_text(renderizar(dados), encoding="utf-8")

    sys.path.insert(0, str(SCRIPTS))
    import validar_documento
    v_erros, v_avisos, *_ = validar_documento.validar(md)

    pdfs = []
    if pdf:
        pdfs.append(gerar_pdf(md, sem_sumario=sem_sumario))
    return md, pdfs, v_erros, v_avisos


def importar(projeto, origem, substituir=False):
    """Carrega um JSON completo (ex.: o exemplo), conferindo cada item pelo mesmo caminho do 'inserir'."""
    bruto = json.loads(Path(origem).expanduser().read_text(encoding="utf-8"))
    if bruto.get("esquema") != ESQUEMA:
        raise ErroMemoria(f"esquema desconhecido: {bruto.get('esquema')}")
    if arquivo_json(projeto).exists() and not substituir:
        raise ErroMemoria(f"projeto '{projeto}' já existe; use --substituir para sobrescrever")
    dados = vazio(projeto, bruto["documento"]["titulo"])
    for secao in OBJETOS:
        novo = {**OBJETOS[secao], **bruto.get(secao, {})}
        checar_item(secao, novo)
        dados[secao] = novo
    for v in bruto.get("versoes", []):
        if not SEMVER.match(v.get("versao", "")) or not DATA.match(v.get("data", "")):
            raise ErroMemoria(f"versão inválida no controle de versões: {v}")
        if dados["versoes"] and tuple(map(int, v["versao"].split("."))) <= tuple(
                map(int, dados["versoes"][-1]["versao"].split("."))):
            raise ErroMemoria(f"versão {v['versao']} não é maior que a anterior")
        dados["versoes"].append({k: v[k] for k in ("versao", "data", "autor", "mudancas")})
    if not dados["versoes"]:
        raise ErroMemoria("o JSON importado não tem controle de versões")
    for secao in LISTAS:
        for item in sorted(bruto.get(secao, []), key=lambda i: i.get("id", "")):
            inserir(dados, secao, item)
    for prefixo, valor in bruto.get("contadores", {}).items():
        dados["contadores"][prefixo] = max(dados["contadores"].get(prefixo, 0), int(valor))
    salvar(projeto, dados)
    return arquivo_json(projeto)


# ---------------------------------------------------------------- linha de comando

def imprimir_json(valor):
    print(json.dumps(valor, ensure_ascii=False, indent=2))


def montar_parser():
    p = argparse.ArgumentParser(description="Memória JSON do terraform-requisitos.")
    sub = p.add_subparsers(dest="comando", required=True)

    s = sub.add_parser("iniciar", help="cria ~/.terraforme/<projeto>/requisitos.json")
    s.add_argument("projeto")
    s.add_argument("--titulo", required=True, help="nome da aplicação, ex.: 'Loja Online'")
    s.add_argument("--autor", default="—")
    s.add_argument("--data")

    s = sub.add_parser("inserir", help="adiciona um item a uma seção de lista")
    s.add_argument("projeto")
    s.add_argument("secao")
    s.add_argument("--json", required=True)

    s = sub.add_parser("editar", help="altera só os campos informados")
    s.add_argument("projeto")
    s.add_argument("secao")
    s.add_argument("id", nargs="?")
    s.add_argument("--campo")
    s.add_argument("--valor", help="texto ou JSON (lista, objeto, null)")
    s.add_argument("--json", help="objeto com os campos a mesclar")
    s.add_argument("--anexar", action="store_true", help="acrescenta o valor a um campo de lista")

    s = sub.add_parser("ler", help="mostra o JSON inteiro, uma seção, um item ou um campo")
    s.add_argument("projeto")
    s.add_argument("secao", nargs="?")
    s.add_argument("id", nargs="?")
    s.add_argument("--campo")

    s = sub.add_parser("buscar", help="procura um texto em todos os campos")
    s.add_argument("projeto")
    s.add_argument("texto")
    s.add_argument("--secao")
    s.add_argument("--json", action="store_true", help="saída em JSON")

    s = sub.add_parser("remover", help="marca RF/RN/RNF como removido ou apaga itens das outras seções")
    s.add_argument("projeto")
    s.add_argument("secao")
    s.add_argument("id")
    s.add_argument("--versao", help="versão em que foi removido (padrão: a última)")

    s = sub.add_parser("versao", help="registra uma nova versão no controle de versões")
    s.add_argument("projeto")
    s.add_argument("--tipo", required=True, choices=["maior", "menor", "correcao"])
    s.add_argument("--mudancas", required=True)
    s.add_argument("--autor")
    s.add_argument("--data")

    s = sub.add_parser("verificar", help="confere as ligações entre seções")
    s.add_argument("projeto")

    s = sub.add_parser("gerar", help="gera o .md (e o PDF) a partir do JSON")
    s.add_argument("projeto")
    s.add_argument("--md", help="caminho do .md (padrão: ~/.terraforme/<projeto>/requisitos.md)")
    s.add_argument("--pdf", action="store_true", help="gera também o PDF")
    s.add_argument("--sem-sumario", action="store_true")

    s = sub.add_parser("importar", help="carrega um requisitos.json completo")
    s.add_argument("projeto")
    s.add_argument("--arquivo", required=True)
    s.add_argument("--substituir", action="store_true")

    s = sub.add_parser("caminho", help="mostra onde fica o JSON do projeto")
    s.add_argument("projeto")
    return p


def executar(args):
    cmd = args.comando
    if cmd == "iniciar":
        print(f"OK: memória criada em {iniciar(args.projeto, args.titulo, args.autor, args.data)}")
        return 0
    if cmd == "caminho":
        print(arquivo_json(args.projeto))
        return 0
    if cmd == "importar":
        print(f"OK: importado em {importar(args.projeto, args.arquivo, args.substituir)}")
        return 0
    if cmd == "gerar":
        md, pdfs, erros, avisos = gerar(args.projeto, args.md, args.pdf, args.sem_sumario)
        print(f"OK: Markdown gerado em {md}")
        for e in erros:
            print(f"ERRO (validador): {e}")
        for a in avisos:
            print(f"AVISO (validador): {a}")
        for pdf in pdfs:
            print(f"OK: PDF gerado em {pdf}")
        return 1 if erros else 0

    dados = carregar(args.projeto)
    secao = secao_canonica(args.secao) if getattr(args, "secao", None) else None

    if cmd == "ler":
        valor = ler(dados, secao, args.id, args.campo)
        print(valor if isinstance(valor, str) else json.dumps(valor, ensure_ascii=False, indent=2))
        return 0
    if cmd == "buscar":
        achados = buscar(dados, args.texto, secao)
        if args.json:
            imprimir_json([{"secao": s, "id": i, "campo": c, "valor": v} for s, i, c, v in achados])
        elif not achados:
            print("Nada encontrado.")
        for s, i, c, v in ([] if args.json else achados):
            print(f"{s}{'/' + i if i else ''} · {c}: {v}")
        return 0
    if cmd == "verificar":
        erros, avisos = verificar(dados)
        for e in erros:
            print(f"ERRO: {e}")
        for a in avisos:
            print(f"AVISO: {a}")
        if not erros:
            print("OK: memória consistente" + (" (com avisos)" if avisos else ""))
        return 1 if erros else 0

    if cmd == "inserir":
        id_ = inserir(dados, secao, ler_json_arg(args.json))
        salvar(args.projeto, dados)
        print(id_)
        return 0
    if cmd == "editar":
        if args.json is not None and args.campo is None:
            mudancas = ler_json_arg(args.json)
        elif args.campo is not None and args.valor is not None and args.json is None:
            mudancas = {args.campo: interpretar_valor(args.valor)}
        else:
            raise ErroMemoria("use --campo com --valor, ou só --json")
        editar(dados, secao, args.id, mudancas, args.anexar)
        salvar(args.projeto, dados)
        print(f"OK: {secao}{'/' + args.id if args.id else ''} atualizado ({', '.join(mudancas)})")
        return 0
    if cmd == "remover":
        resultado = remover(dados, secao, args.id, args.versao)
        salvar(args.projeto, dados)
        print(f"OK: {args.id} {resultado}")
        return 0
    if cmd == "versao":
        versao = nova_versao(dados, args.tipo, args.mudancas, args.autor, args.data)
        salvar(args.projeto, dados)
        print(versao)
        return 0
    return 2


def main(argv=None):
    args = montar_parser().parse_args(argv)
    try:
        return executar(args)
    except ErroMemoria as e:
        print(f"ERRO: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
