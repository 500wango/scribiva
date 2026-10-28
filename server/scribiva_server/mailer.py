"""Outbound mail for magic-link login (FR-019), standard library only.

Production uses SMTP (``SMTP_*``). For local development ``SCRIBIVA_MAIL_OUTBOX``
writes messages as private ``.eml`` files instead; links are never logged.
"""
from __future__ import annotations
import os
import secrets
import smtplib
import ssl
from email.message import EmailMessage
from pathlib import Path

from .config import Settings


class MailError(RuntimeError):
    pass


def login_message(settings: Settings, *, to: str, link: str, user_code: str) -> EmailMessage:
    minutes = max(1, settings.magic_link_ttl_seconds // 60)
    msg = EmailMessage()
    msg['Subject'] = f'Scribiva 登录确认 / Sign in to Scribiva ({user_code})'
    msg['From'] = settings.smtp_from or 'Scribiva <no-reply@localhost>'
    msg['To'] = to
    msg.set_content(
        f'请打开下面的链接完成 Scribiva 登录（{minutes} 分钟内有效，仅可使用一次）：\n'
        f'{link}\n\n'
        f'确认页面显示的代码应为 {user_code}，与桌面应用中显示的一致。\n'
        '如果不是你本人发起的登录，请忽略这封邮件。\n\n'
        f'Open this link to sign in to Scribiva (valid for {minutes} minutes, single use):\n'
        f'{link}\n\n'
        f'The confirmation page should show code {user_code}, matching the desktop app.\n'
        'If you did not request this, ignore this email.\n')
    return msg


def send(settings: Settings, msg: EmailMessage) -> None:
    if settings.smtp_host and settings.smtp_from:
        try:
            if settings.smtp_tls == 'ssl':
                client = smtplib.SMTP_SSL(settings.smtp_host, settings.smtp_port, timeout=15,
                                          context=ssl.create_default_context())
            else:
                client = smtplib.SMTP(settings.smtp_host, settings.smtp_port, timeout=15)
            with client:
                if settings.smtp_tls == 'starttls':
                    client.starttls(context=ssl.create_default_context())
                if settings.smtp_username:
                    client.login(settings.smtp_username, settings.smtp_password)
                client.send_message(msg)
        except (OSError, smtplib.SMTPException) as exc:
            raise MailError('Mail delivery failed') from exc
        return
    if settings.mail_outbox:
        folder = Path(settings.mail_outbox)
        folder.mkdir(parents=True, exist_ok=True)
        os.chmod(folder, 0o700)
        path = folder / f'{secrets.token_hex(8)}.eml'
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'wb') as fh:
            fh.write(bytes(msg))
        return
    raise MailError('Mail is not configured')
