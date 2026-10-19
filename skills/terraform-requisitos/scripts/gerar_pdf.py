#!/usr/bin/env python3
# /// script
# requires-python = ">=3.9"
# dependencies = ["reportlab>=4.0", "markdown-it-py>=3.0"]
# ///
"""Gera um PDF a partir de um documento de requisitos em Markdown.

Uso:
    uv run gerar_pdf.py <documento.md> [-o saida.pdf] [--sem-sumario]

Sem uv, instale as dependências (pip install reportlab markdown-it-py) e rode com python3.
O PDF sai ao lado do .md com o mesmo nome, a menos que -o seja informado.
"""
import argparse
import re
import sys
from datetime import date
from pathlib import Path
from xml.sax.saxutils import escape

try:
    from markdown_it import MarkdownIt
    from reportlab.lib import colors
    from reportlab.lib.enums import TA_CENTER
    from reportlab.lib.pagesizes import A4
    from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
    from reportlab.lib.units import cm
    from reportlab.pdfbase import pdfmetrics
    from reportlab.pdfbase.ttfonts import TTFont
    from reportlab.pdfgen import canvas
    from reportlab.platypus import (
        BaseDocTemplate, Frame, HRFlowable, PageBreak, PageTemplate, Paragraph,
        Preformatted, Spacer, Table, TableStyle,
    )
    from reportlab.platypus.tableofcontents import TableOfContents
except ImportError as e:
    print(f"ERRO: dependência ausente ({e.name}). Rode com 'uv run' ou instale: pip install reportlab markdown-it-py")
    sys.exit(2)

COR_PRIMARIA = colors.HexColor("#1f3a5f")
COR_TEXTO = colors.HexColor("#222222")
COR_SUAVE = colors.HexColor("#6b7280")
COR_BORDA = colors.HexColor("#d1d5db")
COR_FUNDO = colors.HexColor("#f3f4f6")

FONTES_CANDIDATAS = [
    # (regular, negrito, itálico, negrito-itálico, mono)
    ("/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf", "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
     "/usr/share/fonts/truetype/noto/NotoSans-Italic.ttf", "/usr/share/fonts/truetype/noto/NotoSans-BoldItalic.ttf",
     "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf"),
    ("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
     "/usr/share/fonts/truetype/dejavu/DejaVuSans-Oblique.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans-BoldOblique.ttf",
     "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"),
    ("C:/Windows/Fonts/arial.ttf", "C:/Windows/Fonts/arialbd.ttf", "C:/Windows/Fonts/ariali.ttf",
     "C:/Windows/Fonts/arialbi.ttf", "C:/Windows/Fonts/consola.ttf"),
    ("/Library/Fonts/Arial Unicode.ttf",) * 4 + ("/System/Library/Fonts/Menlo.ttc",),
]


def registrar_fontes():
    """Registra uma fonte TrueType com acentos e setas; cai para Helvetica se não achar."""
    for regular, negrito, italico, negrito_italico, mono in FONTES_CANDIDATAS:
        if all(Path(p).exists() for p in (regular, negrito, italico, negrito_italico)):
            pdfmetrics.registerFont(TTFont("Doc", regular))
            pdfmetrics.registerFont(TTFont("Doc-B", negrito))
            pdfmetrics.registerFont(TTFont("Doc-I", italico))
            pdfmetrics.registerFont(TTFont("Doc-BI", negrito_italico))
            pdfmetrics.registerFontFamily("Doc", normal="Doc", bold="Doc-B", italic="Doc-I", boldItalic="Doc-BI")
            if Path(mono).exists() and not mono.endswith(".ttc"):
                pdfmetrics.registerFont(TTFont("DocMono", mono))
                return "Doc", "Doc-B", "DocMono"
            return "Doc", "Doc-B", "Courier"
    print("AVISO: nenhuma fonte TrueType encontrada; usando Helvetica (setas e alguns símbolos podem sumir)")
    return "Helvetica", "Helvetica-Bold", "Courier"


def criar_estilos(fonte, fonte_negrito, fonte_mono):
    base = getSampleStyleSheet()
    s = {}
    s["corpo"] = ParagraphStyle("corpo", parent=base["Normal"], fontName=fonte, fontSize=10, leading=14,
                                textColor=COR_TEXTO, spaceAfter=6)
    s["titulo"] = ParagraphStyle("titulo", parent=s["corpo"], fontName=fonte_negrito, fontSize=22, leading=28,
                                 textColor=COR_PRIMARIA, spaceAfter=4)
    s["h2"] = ParagraphStyle("h2", parent=s["corpo"], fontName=fonte_negrito, fontSize=15, leading=20,
                             textColor=COR_PRIMARIA, spaceBefore=16, spaceAfter=8)
    s["h3"] = ParagraphStyle("h3", parent=s["corpo"], fontName=fonte_negrito, fontSize=12, leading=16,
                             textColor=COR_PRIMARIA, spaceBefore=12, spaceAfter=6)
    s["h4"] = ParagraphStyle("h4", parent=s["corpo"], fontName=fonte_negrito, fontSize=10.5, leading=14,
                             spaceBefore=8, spaceAfter=4)
    s["citacao"] = ParagraphStyle("citacao", parent=s["corpo"], textColor=COR_SUAVE, fontSize=9.5, leading=13,
                                  spaceAfter=3)
    s["celula"] = ParagraphStyle("celula", parent=s["corpo"], fontSize=8.5, leading=11, spaceAfter=0)
    s["celula_cab"] = ParagraphStyle("celula_cab", parent=s["celula"], fontName=fonte_negrito,
                                     textColor=colors.white)
    s["codigo"] = ParagraphStyle("codigo", parent=s["corpo"], fontName=fonte_mono, fontSize=8.5, leading=11,
                                 backColor=COR_FUNDO, borderPadding=6, spaceBefore=4, spaceAfter=10)
    s["sumario_titulo"] = ParagraphStyle("sumario_titulo", parent=s["h2"], spaceBefore=0)
    s["meta"] = ParagraphStyle("meta", parent=s["corpo"], textColor=COR_SUAVE, fontSize=9.5, spaceAfter=14)
    s["toc1"] = ParagraphStyle("toc1", parent=s["corpo"], fontSize=10, leading=16, leftIndent=0)
    s["toc2"] = ParagraphStyle("toc2", parent=s["corpo"], fontSize=9, leading=13, leftIndent=14,
                               textColor=COR_SUAVE)
    s["mono"] = fonte_mono
    return s


def inline_para_markup(token, fonte_mono):
    """Converte os filhos de um token 'inline' do markdown-it em marcação de Paragraph do reportlab."""
    partes = []
    for filho in token.children or []:
        t = filho.type
        if t == "text":
            partes.append(escape(filho.content))
        elif t == "code_inline":
            partes.append(f'<font face="{fonte_mono}" backColor="#eef0f3">{escape(filho.content)}</font>')
        elif t == "strong_open":
            partes.append("<b>")
        elif t == "strong_close":
            partes.append("</b>")
        elif t == "em_open":
            partes.append("<i>")
        elif t == "em_close":
            partes.append("</i>")
        elif t == "s_open":
            partes.append("<strike>")
        elif t == "s_close":
            partes.append("</strike>")
        elif t == "link_open":
            href = escape(filho.attrGet("href") or "", {'"': "&quot;"})
            partes.append(f'<a href="{href}" color="#1d4ed8">')
        elif t == "link_close":
            partes.append("</a>")
        elif t == "softbreak":
            partes.append(" ")
        elif t == "hardbreak":
            partes.append("<br/>")
        elif t == "image":
            partes.append(escape(f"[imagem: {filho.content or filho.attrGet('src')}]"))
        elif t == "html_inline":
            partes.append(escape(filho.content))
    return "".join(partes)


def texto_puro(token):
    return "".join(f.content for f in (token.children or []) if f.type in ("text", "code_inline"))


class Documento(BaseDocTemplate):
    def __init__(self, caminho, titulo, versao, **kw):
        super().__init__(str(caminho), pagesize=A4, leftMargin=2 * cm, rightMargin=2 * cm,
                         topMargin=2.3 * cm, bottomMargin=2 * cm, title=titulo, **kw)
        self.titulo_doc = titulo
        self.versao_doc = versao
        frame = Frame(self.leftMargin, self.bottomMargin, self.width, self.height, id="normal")
        self.addPageTemplates([PageTemplate(id="pagina", frames=[frame], onPage=self._cabecalho)])

    def _cabecalho(self, canv, doc):
        canv.saveState()
        canv.setFont(ESTILOS["corpo"].fontName, 8)
        canv.setFillColor(COR_SUAVE)
        topo = A4[1] - 1.3 * cm
        canv.drawString(self.leftMargin, topo, self.titulo_doc)
        if self.versao_doc:
            canv.drawRightString(A4[0] - self.rightMargin, topo, f"Versão {self.versao_doc}")
        canv.setStrokeColor(COR_BORDA)
        canv.line(self.leftMargin, topo - 4, A4[0] - self.rightMargin, topo - 4)
        canv.restoreState()

    def afterFlowable(self, flowable):
        """Alimenta o sumário e os marcadores do PDF a partir dos títulos."""
        nivel = getattr(flowable, "_nivel_titulo", None)
        if nivel is None:
            return
        texto = flowable.getPlainText()
        chave = f"t{id(flowable)}"
        self.canv.bookmarkPage(chave)
        self.canv.addOutlineEntry(texto, chave, level=nivel, closed=nivel > 0)
        self.notify("TOCEntry", (nivel, texto, self.page, chave))


class CanvasNumerado(canvas.Canvas):
    """Canvas que escreve 'Página X de Y' no rodapé depois de conhecer o total de páginas."""

    def __init__(self, *args, **kw):
        super().__init__(*args, **kw)
        self._paginas = []

    def showPage(self):
        self._paginas.append(dict(self.__dict__))
        self._startPage()

    def save(self):
        total = len(self._paginas)
        for estado in self._paginas:
            self.__dict__.update(estado)
            self.saveState()
            self.setFont(ESTILOS["corpo"].fontName, 8)
            self.setFillColor(COR_SUAVE)
            self.drawCentredString(A4[0] / 2, 1.2 * cm, f"Página {self._pageNumber} de {total}")
            self.restoreState()
            super().showPage()
        super().save()


def larguras_colunas(linhas, largura_total):
    """Garante que a maior palavra de cada coluna caiba inteira e distribui o resto pelo tamanho do conteúdo."""
    n = max(len(l) for l in linhas)
    est_cab, est_cel = ESTILOS["celula_cab"], ESTILOS["celula"]
    folga = 12  # padding esquerdo + direito + margem
    minimos, pesos = [], []
    for c in range(n):
        maior_palavra = 0
        tamanhos = []
        for i, linha in enumerate(linhas):
            texto = linha[c] if c < len(linha) else ""
            est = est_cab if i == 0 else est_cel
            for palavra in texto.split():
                maior_palavra = max(maior_palavra, pdfmetrics.stringWidth(palavra, est.fontName, est.fontSize))
            tamanhos.append(len(texto))
        minimos.append(maior_palavra + folga)
        pesos.append(max(6, min(60, max(tamanhos))))
    sobra = largura_total - sum(minimos)
    if sobra <= 0:  # nem as palavras cabem: reparte proporcionalmente aos mínimos
        return [largura_total * m / sum(minimos) for m in minimos]
    soma = sum(pesos)
    return [m + sobra * p / soma for m, p in zip(minimos, pesos)]


def montar_tabela(linhas_markup, linhas_texto, alinhamentos, largura):
    est = ESTILOS
    dados = []
    for i, linha in enumerate(linhas_markup):
        estilo = est["celula_cab"] if i == 0 else est["celula"]
        dados.append([Paragraph(c, estilo) for c in linha])
    n = max(len(l) for l in dados)
    for linha in dados:
        linha.extend([Paragraph("", est["celula"])] * (n - len(linha)))
    tabela = Table(dados, colWidths=larguras_colunas(linhas_texto, largura), repeatRows=1, hAlign="LEFT")
    comandos = [
        ("BACKGROUND", (0, 0), (-1, 0), COR_PRIMARIA),
        ("GRID", (0, 0), (-1, -1), 0.4, COR_BORDA),
        ("VALIGN", (0, 0), (-1, -1), "TOP"),
        ("TOPPADDING", (0, 0), (-1, -1), 4),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 4),
        ("LEFTPADDING", (0, 0), (-1, -1), 5),
        ("RIGHTPADDING", (0, 0), (-1, -1), 5),
    ]
    for i in range(2, len(dados), 2):
        comandos.append(("BACKGROUND", (0, i), (-1, i), COR_FUNDO))
    for c, alinhamento in enumerate(alinhamentos):
        if alinhamento in ("center", "right"):
            comandos.append(("ALIGN", (c, 0), (c, -1), alinhamento.upper()))
    tabela.setStyle(TableStyle(comandos))
    return [tabela, Spacer(1, 10)]


def markdown_para_flowables(texto_md, largura):
    est = ESTILOS
    md = MarkdownIt("commonmark").enable(["table", "strikethrough"])
    tokens = md.parse(texto_md)

    historia = []
    titulo = None
    listas = []          # pilha de [tipo, contador]
    primeiro_do_item = []
    citacao = 0
    tabela = None        # (markup, texto, alinhamentos)
    linha_tab = None
    i = 0
    while i < len(tokens):
        tok = tokens[i]
        t = tok.type

        if t == "heading_open":
            nivel = int(tok.tag[1])
            inline = tokens[i + 1]
            markup = inline_para_markup(inline, est["mono"])
            if nivel == 1 and titulo is None:
                titulo = texto_puro(inline)
                p = Paragraph(markup, est["titulo"])
            else:
                p = Paragraph(markup, est.get(f"h{min(nivel, 4)}", est["h4"]))
                if nivel in (2, 3):
                    p._nivel_titulo = nivel - 2
            historia.append(p)
            if nivel == 1:
                historia.append(HRFlowable(width="100%", thickness=1.2, color=COR_PRIMARIA, spaceAfter=10))
            i += 3
            continue

        if t in ("bullet_list_open", "ordered_list_open"):
            inicio = int(tok.attrGet("start") or 1)
            listas.append(["ul" if t == "bullet_list_open" else "ol", inicio - 1])
        elif t in ("bullet_list_close", "ordered_list_close"):
            listas.pop()
            if not listas:
                historia.append(Spacer(1, 4))
        elif t == "list_item_open":
            listas[-1][1] += 1
            primeiro_do_item.append(True)
        elif t == "list_item_close":
            primeiro_do_item.pop()

        elif t == "blockquote_open":
            citacao += 1
        elif t == "blockquote_close":
            citacao -= 1

        elif t == "table_open":
            tabela = ([], [], [])
        elif t == "tr_open":
            linha_tab = ([], [])
        elif t in ("th_open", "td_open"):
            inline = tokens[i + 1]
            linha_tab[0].append(inline_para_markup(inline, est["mono"]))
            linha_tab[1].append(texto_puro(inline))
            if t == "th_open":
                estilo_css = tok.attrGet("style") or ""
                m = re.search(r"text-align:(\w+)", estilo_css)
                tabela[2].append(m.group(1) if m else "left")
            i += 3
            continue
        elif t == "tr_close":
            tabela[0].append(linha_tab[0])
            tabela[1].append(linha_tab[1])
        elif t == "table_close":
            recuo = 18 * len(listas)
            historia.extend(montar_tabela(tabela[0], tabela[1], tabela[2], largura - recuo))
            tabela = None

        elif t == "inline" and tabela is None:
            markup = inline_para_markup(tok, est["mono"])
            estilo = est["citacao"] if citacao else est["corpo"]
            if listas:
                profundidade = len(listas)
                tipo, contador = listas[-1]
                marcador = f"{contador}." if tipo == "ol" else ("•" if profundidade % 2 else "◦")
                estilo = ParagraphStyle(f"lista{profundidade}", parent=estilo, leftIndent=16 * profundidade,
                                        bulletIndent=16 * profundidade - 11, spaceAfter=3)
                if primeiro_do_item and primeiro_do_item[-1]:
                    historia.append(Paragraph(markup, estilo, bulletText=marcador))
                    primeiro_do_item[-1] = False
                else:
                    historia.append(Paragraph(markup, estilo))
            elif citacao:
                caixa = Table([[Paragraph(markup, estilo)]], colWidths=[largura], hAlign="LEFT")
                caixa.setStyle(TableStyle([
                    ("LINEBEFORE", (0, 0), (0, -1), 2.5, COR_PRIMARIA),
                    ("BACKGROUND", (0, 0), (-1, -1), COR_FUNDO),
                    ("LEFTPADDING", (0, 0), (-1, -1), 8),
                    ("TOPPADDING", (0, 0), (-1, -1), 5),
                    ("BOTTOMPADDING", (0, 0), (-1, -1), 3),
                ]))
                historia.extend([caixa, Spacer(1, 6)])
            else:
                historia.append(Paragraph(markup, estilo))

        elif t in ("fence", "code_block"):
            historia.append(Preformatted(tok.content.rstrip("\n"), est["codigo"]))
        elif t == "hr":
            historia.append(HRFlowable(width="100%", thickness=0.6, color=COR_BORDA, spaceBefore=6, spaceAfter=10))
        elif t == "html_block":
            historia.append(Paragraph(escape(tok.content), est["corpo"]))

        i += 1

    return historia, titulo


def gerar(entrada, saida, com_sumario=True):
    texto = entrada.read_text(encoding="utf-8")
    m = re.search(r"Vers[aã]o do documento:\**\s*(\d+\.\d+\.\d+)", texto, re.IGNORECASE)
    versao = m.group(1) if m else None

    doc_largura = A4[0] - 4 * cm
    historia, titulo = markdown_para_flowables(texto, doc_largura)
    titulo = titulo or entrada.stem

    # Bloco de abertura: título, metadados e sumário em página própria.
    meta = f"Gerado em {date.today():%d/%m/%Y}" + (f" · Versão {versao}" if versao else "")
    abertura_fim = 2 if historia and isinstance(historia[1], HRFlowable) else 1
    abertura = historia[:abertura_fim] + [Paragraph(meta, ESTILOS["meta"])]
    corpo = historia[abertura_fim:]
    if com_sumario:
        sumario = TableOfContents()
        sumario.levelStyles = [ESTILOS["toc1"], ESTILOS["toc2"]]
        sumario.dotsMinLevel = 0
        abertura += [Paragraph("Sumário", ESTILOS["sumario_titulo"]), sumario, PageBreak()]

    doc = Documento(saida, titulo, versao, author="terraform-requisitos")
    doc.multiBuild(abertura + corpo, canvasmaker=CanvasNumerado)
    return saida


def main():
    parser = argparse.ArgumentParser(description="Gera PDF a partir de um documento de requisitos em Markdown.")
    parser.add_argument("entrada", type=Path, help="arquivo .md de entrada")
    parser.add_argument("-o", "--saida", type=Path, help="arquivo .pdf de saída (padrão: mesmo nome do .md)")
    parser.add_argument("--sem-sumario", action="store_true", help="não gerar a página de sumário")
    args = parser.parse_args()

    entrada = args.entrada.expanduser()
    if not entrada.is_file():
        print(f"ERRO: arquivo não encontrado: {entrada}")
        sys.exit(2)
    saida = (args.saida or entrada.with_suffix(".pdf")).expanduser()
    saida.parent.mkdir(parents=True, exist_ok=True)

    global ESTILOS
    ESTILOS = criar_estilos(*registrar_fontes())
    gerar(entrada, saida, com_sumario=not args.sem_sumario)
    print(f"OK: PDF gerado em {saida}")


ESTILOS = {}

if __name__ == "__main__":
    main()
