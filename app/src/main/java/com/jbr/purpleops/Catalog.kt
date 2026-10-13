package com.jbr.purpleops

import org.json.JSONObject

data class ParamDef(val name: String, val hint: String, val def: String)
data class ToolAction(val label: String, val cmd: String, val params: List<ParamDef>)
data class Tool(val name: String, val desc: String, val actions: List<ToolAction>)
data class Category(val name: String, val tools: List<Tool>)

object Catalog {
    fun parse(raw: String): List<Category> {
        val root = JSONObject(raw)
        val cats = mutableListOf<Category>()
        val catArr = root.optJSONArray("categories") ?: return cats
        for (i in 0 until catArr.length()) {
            val c = catArr.getJSONObject(i)
            val tools = mutableListOf<Tool>()
            val toolArr = c.optJSONArray("tools")
            if (toolArr != null) for (j in 0 until toolArr.length()) {
                val t = toolArr.getJSONObject(j)
                val actions = mutableListOf<ToolAction>()
                val actArr = t.optJSONArray("actions")
                if (actArr != null) for (k in 0 until actArr.length()) {
                    val a = actArr.getJSONObject(k)
                    val params = mutableListOf<ParamDef>()
                    val pArr = a.optJSONArray("params")
                    if (pArr != null) for (m in 0 until pArr.length()) {
                        val p = pArr.getJSONObject(m)
                        params.add(
                            ParamDef(
                                p.getString("name"),
                                p.optString("hint", p.getString("name")),
                                p.optString("default", "")
                            )
                        )
                    }
                    actions.add(ToolAction(a.getString("label"), a.getString("cmd"), params))
                }
                tools.add(Tool(t.getString("name"), t.optString("desc", ""), actions))
            }
            cats.add(Category(c.getString("name"), tools))
        }
        return cats
    }
}
