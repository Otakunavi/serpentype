"""Synthetic GeneralNoticeCase data for local PDF rendering."""


class GeneralNoticeCase:
    """Return the GeneralNoticeCase context shape with local sample values."""

    @staticmethod
    def get_document_dict(xml_qr_ru: str, xml_qr_kk: str) -> dict[str, dict]:
        appendix = {
            "violation_sum": "123 457",
            "row_number": 1,
            "fno_code": "100.00",
            "period": "01.01.2025 - 31.01.2025",
            "violation_text": "Тестовое расхождение",
            "table_name": "Тестовая таблица",
            "table_insert": (
                '<table style="border-collapse:collapse;width:100%">'
                '<thead><tr><th style="border:1px solid">Показатель</th></tr></thead>'
                '<tbody><tr><td style="border:1px solid">Тестовое значение</td></tr></tbody>'
                "</table>"
            ),
        }

        ru = {
            "page": {"app": {"count": 1}},
            "cameral": {
                "notice": {
                    "customer_tin": "123456789012",
                    "customer_name": "ТОО Тест",
                    "ogd_code": "5900",
                    "ogd_name": "Департамент государственных доходов (тест)",
                    "reg_number": "0001",
                    "reg_date": "05.02.2025",
                    "supervisor": "Тестовый пользователь",
                    "address": "г. Астана, ул. Тестовая, 1",
                    "ogd_supervisor_name": "Руководитель тестового ОГД",
                    "ogd_address": "г. Астана, тестовый адрес ОГД",
                    "customer_address": "г. Астана, ул. Тестовая, 1",
                    "period": "01.01.2025 - 31.01.2025",
                    "fno_code": "100.00",
                    "violation_text": "Тестовое расхождение",
                    "violation_sum": "123 457",
                    "violation_result_date": "05.02.2025",
                },
                "document_type": "NOTICE",
                "doc_code": "7",
                "predecessor_tax_payer_bin": "",
                "previous_document_number": "",
                "previous_document_date": "",
                "is_redirected": False,
                "appendix": [appendix],
                "has_itemization_file": False,
            },
            "PAGE_BREAKER_AFTER": '<p style="page-break-after:always">',
            "signed_xml_qr_codes": xml_qr_ru,
        }

        kk = {
            "page": {"app": {"count": 1}},
            "cameral": {
                "notice": {
                    **ru["cameral"]["notice"],
                    "customer_name": "Тест ЖШС",
                    "ogd_name": "Мемлекеттік кірістер департаменті (сынақ)",
                    "address": "Астана қ., Сынақ көшесі, 1",
                    "ogd_address": "Астана қ., сынақ МКД мекенжайы",
                    "customer_address": "Астана қ., Сынақ көшесі, 1",
                    "violation_text": "Сынақ алшақтығы",
                },
                **{key: value for key, value in ru["cameral"].items() if key != "notice"},
                "appendix": [{
                    **appendix,
                    "violation_text": "Сынақ алшақтығы",
                    "table_name": "Сынақ кестесі",
                    "table_insert": (
                        '<table style="border-collapse:collapse;width:100%">'
                        '<thead><tr><th style="border:1px solid">Көрсеткіш</th></tr></thead>'
                        '<tbody><tr><td style="border:1px solid">Сынақ мәні</td></tr></tbody>'
                        "</table>"
                    ),
                }],
            },
            "PAGE_BREAKER_AFTER": ru["PAGE_BREAKER_AFTER"],
            "signed_xml_qr_codes": xml_qr_kk,
        }
        return {"ru": ru, "kk": kk}
