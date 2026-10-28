"""Minimal server-rendered pages for the browser half of device login.

Self-contained HTML (no external fonts, scripts or CDNs) so it loads from
mainland China. All dynamic values are HTML-escaped.
"""
from __future__ import annotations
from html import escape

_STYLE = """
:root{color-scheme:light dark;--bg:#f7f7f5;--fg:#16181d;--muted:#5b616e;--card:#fff;
--line:#d9dbe0;--accent:#1f5eff;--accent-fg:#fff;--err:#a4161a}
@media (prefers-color-scheme:dark){:root{--bg:#0f1115;--fg:#eceef2;--muted:#a3a9b6;
--card:#181b21;--line:#2c313b;--accent:#6c9bff;--accent-fg:#0b0d12;--err:#ff8a8f}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);
font:16px/1.6 system-ui,-apple-system,"Segoe UI","PingFang SC","Microsoft YaHei",
"Noto Sans CJK SC",sans-serif}main{max-width:440px;margin:8vh auto;padding:0 16px}
.card{background:var(--card);border:1px solid var(--line);border-radius:14px;padding:28px}
h1{font-size:1.35rem;margin:0 0 .5rem}p{margin:.4rem 0 1rem;color:var(--muted)}
label{display:block;font-weight:600;margin:.9rem 0 .3rem}
input{width:100%;padding:.7rem .8rem;border:1px solid var(--line);border-radius:9px;
background:transparent;color:inherit;font:inherit}
input:focus-visible,button:focus-visible{outline:3px solid var(--accent);outline-offset:2px}
button{margin-top:1.2rem;width:100%;padding:.75rem;border:0;border-radius:9px;
background:var(--accent);color:var(--accent-fg);font:inherit;font-weight:700;cursor:pointer}
.code{font:700 1.5rem/1.2 ui-monospace,Consolas,monospace;letter-spacing:.12em}
.error{color:var(--err);font-weight:600}.brand{font-weight:800;margin-bottom:1rem}
"""


def _page(title: str, body: str) -> bytes:
    return ('<!doctype html><html lang="zh-CN"><head><meta charset="utf-8">'
            '<meta name="viewport" content="width=device-width,initial-scale=1">'
            '<meta name="referrer" content="no-referrer">'
            f'<title>{escape(title)}</title><style>{_STYLE}</style></head><body><main>'
            f'<div class="brand">Scribiva</div><div class="card">{body}</div>'
            '</main></body></html>').encode('utf-8')


def login_form(code: str = '', error: str = '') -> bytes:
    err = f'<p class="error" role="alert">{escape(error)}</p>' if error else ''
    return _page('登录 Scribiva / Sign in', f"""
<h1>登录 Scribiva <span lang="en">/ Sign in</span></h1>
<p>输入邮箱，我们会发送一次性登录链接。<span lang="en">Enter your email and we will send a one-time sign-in link.</span></p>
{err}
<form method="post" action="/login">
<label for="code">应用中显示的代码 <span lang="en">/ Code shown in the app</span></label>
<input id="code" name="user_code" required autocomplete="off" inputmode="text"
 pattern="[A-Za-z]{{4}}-?[A-Za-z]{{4}}" value="{escape(code)}">
<label for="email">邮箱 <span lang="en">/ Email</span></label>
<input id="email" name="email" type="email" required autocomplete="email">
<button type="submit">发送登录链接 / Send link</button>
</form>""")


def link_sent() -> bytes:
    return _page('请查收邮件 / Check your email', """
<h1>请查收邮件 <span lang="en">/ Check your email</span></h1>
<p>如果邮箱有效，登录链接已发送，15 分钟内有效。可以关闭此页面。</p>
<p lang="en">If the address is valid, a sign-in link is on its way and is valid for 15 minutes.
You can close this page.</p>""")


def confirm(token: str, user_code: str) -> bytes:
    # A POST is required so that mail scanners prefetching the link cannot
    # approve the login on the user's behalf.
    return _page('确认登录 / Confirm sign-in', f"""
<h1>确认登录 <span lang="en">/ Confirm sign-in</span></h1>
<p>请确认桌面应用中显示的代码与下方一致：</p>
<p class="code" aria-label="code">{escape(user_code)}</p>
<form method="post" action="/auth/verify">
<input type="hidden" name="token" value="{escape(token)}">
<button type="submit">确认并登录 / Confirm</button>
</form>""")


def done() -> bytes:
    return _page('已登录 / Signed in', """
<h1>已登录 <span lang="en">/ Signed in</span></h1>
<p>请回到 Scribiva 应用，它会自动完成登录。可以关闭此页面。</p>
<p lang="en">Return to the Scribiva app; it will finish signing in automatically.</p>""")


def failed(message: str) -> bytes:
    return _page('登录失败 / Sign-in failed', f"""
<h1>登录失败 <span lang="en">/ Sign-in failed</span></h1>
<p class="error" role="alert">{escape(message)}</p>
<p>请回到应用重新开始登录。<span lang="en">Start again from the app.</span></p>""")


def billing_result(kind: str) -> bytes:
    """Landing pages for Stripe Checkout / Billing Portal return URLs. They
    never grant anything: entitlement comes only from signed webhooks."""
    texts = {
        'success': ('支付已提交 / Payment submitted',
                    '<p>感谢订阅。Pro 会在 Stripe 确认付款后几秒内生效，回到 Scribiva 即可看到。</p>'
                    '<p lang="en">Thank you. Pro becomes active a few seconds after Stripe confirms '
                    'the payment; return to Scribiva to see it.</p>'),
        'cancel': ('已取消 / Checkout cancelled',
                   '<p>没有扣款。可以随时从 Scribiva 的“账户与订阅”页面重新订阅。</p>'
                   '<p lang="en">No charge was made. You can subscribe again from Account &amp; '
                   'billing in Scribiva.</p>'),
        'portal': ('已返回 / Back from billing',
                   '<p>订阅设置已保存在 Stripe。回到 Scribiva 查看最新状态。</p>'
                   '<p lang="en">Your subscription settings are saved with Stripe. Return to '
                   'Scribiva to see the current status.</p>'),
    }
    title, body = texts.get(kind, texts['portal'])
    return _page(title, f'<h1>{escape(title)}</h1>{body}<p>可以关闭此页面。You can close this page.</p>')
