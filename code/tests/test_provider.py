# -*- coding: utf-8 -*-
"""provider.py 单元测试:Mock、工厂切换、key 缺失、OpenAI 兼容请求构造(不发真实请求)。"""
import pytest

from llm import provider as P


# ---------------- MockProvider ----------------

# Scenario: mock returns fixed response.
def test_mock_fixed():
    m = P.MockProvider(response="div(ma(close, 20), std(high, 10))")
    assert m.complete("任意 prompt") == "div(ma(close, 20), std(high, 10))"


# Scenario: mock responder overrides response.
def test_mock_responder():
    m = P.MockProvider(responder=lambda prompt: "REJECT: 测试")
    assert m.complete("...").startswith("REJECT")


# ---------------- 工厂切换(能切 provider) ----------------

# Scenario: factory returns mock.
def test_factory_mock():
    assert isinstance(P.get_provider("mock"), P.MockProvider)


# Scenario: factory returns deepseek with preset.
def test_factory_deepseek():
    prov = P.get_provider("deepseek", api_key="sk-test")
    assert isinstance(prov, P.OpenAICompatibleProvider)
    assert prov.base_url == "https://api.deepseek.com"
    assert prov.api_key == "sk-test"
    assert prov.model == "deepseek-chat"


# Scenario: factory returns kimi with preset.
def test_factory_kimi():
    prov = P.get_provider("kimi", api_key="sk-kimi-test")
    assert isinstance(prov, P.OpenAICompatibleProvider)
    assert prov.base_url == "https://api.moonshot.cn/v1"
    assert prov.api_key == "sk-kimi-test"
    assert prov.model == "kimi-latest"


# Scenario: kimi without key raises.
def test_kimi_key(monkeypatch):
    monkeypatch.delenv("KIMI_API_KEY", raising=False)
    with pytest.raises(ValueError):
        P.get_provider("kimi")


# Scenario: kimi reads env key.
def test_kimi_env(monkeypatch):
    monkeypatch.setenv("KIMI_API_KEY", "sk-kimi-from-env")
    prov = P.get_provider("kimi")
    assert prov.api_key == "sk-kimi-from-env"
    assert prov.model == "kimi-latest"


# Scenario: factory returns glm with preset.
def test_factory_glm():
    prov = P.get_provider("glm", api_key="glm-test")
    assert isinstance(prov, P.OpenAICompatibleProvider)
    assert prov.base_url == "https://open.bigmodel.cn/api/paas/v4"
    assert prov.model == "glm-4-plus"


# Scenario: factory unknown raises.
def test_factory_unknown():
    with pytest.raises(ValueError):
        P.get_provider("nope")


# ---------------- key 缺失 ----------------

# Scenario: deepseek without key raises.
def test_deepseek_key(monkeypatch):
    monkeypatch.delenv("DEEPSEEK_API_KEY", raising=False)
    with pytest.raises(ValueError):
        P.get_provider("deepseek")


# Scenario: deepseek reads env key.
def test_deepseek_env(monkeypatch):
    monkeypatch.setenv("DEEPSEEK_API_KEY", "from-env")
    prov = P.get_provider("deepseek")
    assert prov.api_key == "from-env"


# ---------------- OpenAI 兼容请求构造(monkeypatch,不联网) ----------------

class _FakeResp:
    def __init__(self, payload, status_code=200, text=""):
        self._payload = payload
        self.status_code = status_code
        self.text = text or (payload if isinstance(payload, str) else "")
    def raise_for_status(self):
        if self.status_code >= 400:
            raise __import__("requests").HTTPError(f"HTTP {self.status_code}")
    def json(self):
        return self._payload


# Scenario: openai compat request shape.
def test_openai_compat(monkeypatch):
    captured = {}
    def fake_post(url, headers=None, json=None, timeout=None):
        captured["url"] = url
        captured["headers"] = headers
        captured["json"] = json
        return _FakeResp({"choices": [{"message": {"content": "zscore(close)"}}]})
    monkeypatch.setattr(P.requests, "post", fake_post)

    prov = P.get_provider("deepseek", api_key="sk-x")
    out = prov.complete("生成一个因子", temperature=0.9)

    assert out == "zscore(close)"
    assert captured["url"] == "https://api.deepseek.com/chat/completions"
    assert captured["headers"]["Authorization"] == "Bearer sk-x"
    assert captured["json"]["model"] == "deepseek-chat"
    assert captured["json"]["temperature"] == 0.9
    assert captured["json"]["messages"][0]["role"] == "system"
    assert captured["json"]["messages"][1]["content"] == "生成一个因子"


# ---------------- 429 限流退避(2026-08-19,kimi org RPM=3) ----------------

# Scenario: 429 retry then success.
def test_429_retry(monkeypatch):
    calls = []
    def fake_post(url, headers=None, json=None, timeout=None):
        calls.append(url)
        if len(calls) == 1:
            return _FakeResp({"error": {"message": "x"}}, status_code=429,
                             text='{"error":{"message":"max RPM reached, try after 5 seconds"}}')
        return _FakeResp({"choices": [{"message": {"content": "retried-ok"}}]})
    monkeypatch.setattr(P.requests, "post", fake_post)

    prov = P.get_provider("deepseek", api_key="sk-x", retry_on_429=2)
    out = prov.complete("hi")
    assert out == "retried-ok"
    assert len(calls) == 2
    assert prov._429_waits >= 5.0


# Scenario: 429 exhausts retries raises.
def test_429_exhausts(monkeypatch):
    def fake_post(url, headers=None, json=None, timeout=None):
        return _FakeResp({"error": {"message": "rate limited"}}, status_code=429)
    monkeypatch.setattr(P.requests, "post", fake_post)

    prov = P.get_provider("deepseek", api_key="sk-x", retry_on_429=1)
    with pytest.raises(Exception):
        prov.complete("hi")


# Scenario: kimi preset builtin throttle.
def test_kimi_preset():
    prov = P.get_provider("kimi", api_key="sk-kimi-x")
    assert prov.fixed_temperature == 1.0
    assert prov.min_interval >= 25.0
    assert prov.retry_on_429 >= 1
