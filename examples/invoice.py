"""Run after installing the wheel: python examples/invoice.py."""
from pathlib import Path
from serpentype import FontRegistry, Renderer, bundled_font_path

fonts = FontRegistry(cache_bytes=64 * 1024 * 1024)
fonts.register_file(bundled_font_path(), family="Noto Sans")
fonts.register_file(bundled_font_path(700), family="Noto Sans", weight=700)
renderer = Renderer(fonts=fonts)
rows = "".join(f"<tr><td>{i}</td><td>Услуга {i}</td><td>{i * 100} ₽</td></tr>" for i in range(1, 60))
html = f"<h1>Счёт на оплату</h1><p>Позиции заказа:</p><table><thead><tr><th>№</th><th>Описание</th><th>Цена</th></tr></thead><tbody>{rows}</tbody></table>"
css = """@page { size: A4; margin: 18mm }
         h1 { color: #223366 }
         table { width: 170mm }
         th, td { border: 0.5pt solid #666666; padding: 4pt; font-size: 10pt }
         th { background: #eeeeee }
         .pagebreaker { break-after: page }"""
document = renderer.layout(html, css=css)
Path("invoice.pdf").write_bytes(document.to_pdf())
print(f"Created {document.page_count} pages")
