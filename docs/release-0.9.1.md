# Serpentype 0.9.1

Serpentype 0.9.1 keeps documents renderable when an `<img>` uses a `data:` URI
with an unsupported media type. The image is omitted and a source-located
`image-data-uri-media-type` warning is recorded instead of aborting PDF
generation.

Other image data URI errors, resource limits, and local image loading errors
continue to be reported as errors.
