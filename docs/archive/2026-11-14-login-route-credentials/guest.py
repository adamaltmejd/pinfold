"""Host-side generator for one-off curl/sh observations inside a real box."""
import json, shlex

FIELDS = ['Authorization', 'X-Api-Key', 'Api-Key', 'X-Goog-Api-Key', 'X-Auth-Token',
          'X-Access-Token', 'ChatGPT-Account-ID', 'OpenAI-Organization',
          'OpenAI-Project', 'Cookie', 'Cookie2', 'Proxy-Fixture']
QUERIES = ['key', 'api_key', 'api-key', 'apikey', 'x-api-key', 'access_token',
           'refresh_token', 'id_token', 'auth_token', 'bearer_token', 'token',
           'client_secret', 'authorization', 'chatgpt-account-id',
           'openai-organization', 'openai-project']
BODY = b'fixture-body\x00\xff?token=body-is-not-inspected'
PREPARE = "set -eu\nprintf 'fixture-body\\000\\377?token=body-is-not-inspected' > /tmp/pfc-body\n"


def command(route, case, query='', headers=(), body=b'', reflection=False):
    path = '/arbitrary/non-model/' + case + query
    head = [('Host', 'guest.invalid'), ('X-Probe-Id', route + '/' + case),
            ('Content-Length', str(len(body))), *headers]
    argv = ['curl', '--silent', '--show-error', '--max-time', '35', '--globoff',
            '--path-as-is', '--noproxy', '', '--proxy', 'http://127.0.0.1:3128',
            '--request', 'POST', '--data-binary', '@/tmp/pfc-body' if body else '@/dev/null',
            '--output', '/tmp/pfc-response', '--write-out', '%{http_code}']
    for name, value in head:
        argv.extend(['--header', name + ': ' + value])
    argv.append('http://' + route + path)
    pattern = json.dumps([route, case])[:-1] + ',%s]\\n'
    result = 'status=$(' + shlex.join(argv) + ')\n'
    result += 'printf ' + shlex.quote(pattern) + ' "$status"\n'
    if reflection:
        pattern = json.dumps([route, 'reflection-digest'])[:-1] + ',"%s"]\\n'
        result += 'digest=$(sha256sum /tmp/pfc-response); digest=${digest%% *}\n'
        result += 'printf ' + shlex.quote(pattern) + ' "$digest"\n'
    return result


def script(routes):
    parts = [PREPARE]
    def request(*args, **kwargs):
        parts.append(command(*args, **kwargs))
    for route, policy in routes.items():
        hardest = [(name.swapcase(), 'guest-first, guest-second') for name in FIELDS]
        hardest += [(name.lower(), 'guest-third') for name in FIELDS]
        hardest += [('Connection', 'Authorization, X-Trace-Drop, Host'),
                    ('cOnNeCtIoN', ',\t, ChatGPT-Account-ID, '),
                    ('X-Trace-Drop', 'remove-me'), ('X-Trace-Keep', 'keep-first'),
                    ('x-trace-keep', 'keep-second'), ('anthropic-version', '2023-06-01'),
                    ('anthropic-beta', 'beta-one, beta-two')]
        request(route, 'hardest', headers=hardest, body=BODY)
        if policy != 'login':
            request(route, 'ordinary-query', '?token=&api_key&safe=%GG',
                    [('Connection', 'not a token'), ('X-Trace-Drop', 'retain-me'),
                     ('Cookie', 'guest-cookie'), ('Authorization', 'guest-auth'),
                     ('X-Api-Key', 'guest-key')], BODY)
            continue
        request(route, 'unnominated', headers=[(name.swapcase(), 'guest-one, guest-two') for name in FIELDS] + [(name.lower(), 'guest-three') for name in FIELDS], body=BODY)
        request(route, 'harmless', '?safe=%61%2f%3B&plus=a+b;empty=&bare&tokenized=yes', body=BODY)
        request(route, 'decode-once', '?%2574oken=accepted&to+ken=accepted')
        count = 0
        for name in QUERIES:
            spellings = [name, name.upper().replace('_', '-'), name.upper().replace('-', '_'),
                         ''.join('%%%02X' % ord(c) for c in name)]
            for spelling in spellings:
                for separator in ['&', ';']:
                    for field in [spelling + '=value', spelling + '=', spelling,
                                  spelling + '=a' + separator + spelling + '=b']:
                        request(route, 'query-%03d' % count, '?safe=ok' + separator + field)
                        count += 1
        for case, query in [('bad-name', '?safe%'), ('bad-short', '?safe=%0'),
                            ('bad-value', '?safe=%GG'), ('bad-hidden', '?token=x&safe=%GG')]:
            request(route, case, query)
            request(route, case + '-corrected', '?safe=%20')
        for case, options in [('bad-space', 'X Trace'), ('bad-quoted', '"X-Trace"'),
                              ('bad-colon', 'X-Trace:bad')]:
            request(route, case, headers=[('Connection', options)])
            request(route, case + '-corrected', headers=[('Connection', ' ,\t, X-Trace-Drop, '),
                                                       ('X-Trace-Drop', 'remove-me')])
        request(route, 'nominated-length', headers=[('Connection', 'CoNtEnT-LeNgTh')], body=BODY)
        request(route, 'nominated-length-corrected', headers=[('Connection', 'X-Trace-Drop')], body=BODY)
        request(route, 'nominated-beta', headers=[('Connection', 'anthropic-beta'),
                                               ('anthropic-beta', 'remove-me'),
                                               ('anthropic-version', '2023-06-01')])
        request(route, 'reflect', reflection=True)
    parts.append('rm -f /tmp/pfc-body /tmp/pfc-response\n')
    return ''.join(parts)
