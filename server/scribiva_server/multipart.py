"""Small multipart/form-data parser built on the stdlib email package."""
from __future__ import annotations
from email.parser import BytesParser
from email.policy import default


class MultipartError(ValueError):
    pass


MAX_FIELDS = 20
MAX_FIELD_BYTES = 4096


def parse_multipart(body: bytes, content_type: str) -> tuple[dict[str, str], dict[str, dict]]:
    if 'multipart/form-data' not in content_type.lower():
        raise MultipartError('Expected multipart/form-data')
    # The email parser needs a MIME header block; the body is already bounded by
    # the HTTP layer, and policy.default tolerates arbitrary UTF-8 payloads.
    header = (f'Content-Type: {content_type}\r\nMIME-Version: 1.0\r\n\r\n').encode('latin-1')
    message = BytesParser(policy=default).parsebytes(header + body)
    if not message.is_multipart():
        raise MultipartError('Malformed multipart body')
    fields: dict[str, str] = {}
    files: dict[str, dict] = {}
    seen = 0
    for part in message.iter_parts():
        seen += 1
        if seen > MAX_FIELDS:
            raise MultipartError('Too many multipart fields')
        disposition = part.get('Content-Disposition', '')
        if 'form-data' not in disposition.lower():
            continue
        try:
            name = part.get_param('name', header='content-disposition')
        except Exception as exc:  # malformed Content-Disposition
            raise MultipartError('Invalid form field header') from exc
        if not name or not isinstance(name, str):
            continue
        payload = part.get_payload(decode=True) or b''
        filename = part.get_filename()
        if filename or part.get_content_maintype() != 'text':
            if name in files:
                raise MultipartError('Duplicate file field: ' + name)
            files[name] = {
                'filename': filename or '',
                'content_type': part.get_content_type(),
                'data': payload,
            }
        else:
            if name in fields:
                raise MultipartError('Duplicate field: ' + name)
            if len(payload) > MAX_FIELD_BYTES:
                raise MultipartError('Form field is too long')
            fields[name] = payload.decode('utf-8', errors='replace').strip()
    return fields, files
