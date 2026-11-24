# Serpentype 0.11.1

- Keep a CSS Grid row together when it fits on the next page. This prevents
  notice rendering from failing when the XML QR row reaches a page break.
- Render HTML `<figure>` and `<figcaption>` as block containers, preserving
  the top QR image and caption in the original Russian and Kazakh templates.
- Add regressions for row pagination and images nested in a figure.
