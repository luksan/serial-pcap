---
---
local x328_proto = Proto("x328", "X3.28 field bus")

x328_proto.fields.address = ProtoField.string("x328.address", "Address")
x328_proto.fields.parameter = ProtoField.string("x328.parameter", "Parameter")
x328_proto.fields.value = ProtoField.string("x328.value", "Value")

local param_field = Field.new("x328.parameter")

function x328_proto.dissector(tvb, pinfo, tree)

    pinfo.cols.protocol= "X3.28"
    local tree = tree:add(x328_proto, tvb(), "X3.28 field bus")
    if pinfo.src == Address.ip("127.0.0.1") then
        dissect_master(tvb, pinfo, tree)
    else
        pinfo.cols.info = "Reply:  "
    end
end

function dissect_master(tvb, pinfo, tree)
    pinfo.cols.info = "Master: "
    tree:add(x328_proto.fields.address, tvb(1,4))
    if tvb(5,1):uint() == 2 then -- write command
        local param = tree:add(x328_proto.fields.parameter, tvb(6,4))
        local value_len = 0
        for i = 1, 6,1 do
            if tvb(9+i,1):uint() == 3 then
                value_len = i-1
                break
            end
        end
        tree:add(x328_proto.fields.value, tvb(10, value_len))

        pinfo.cols.info = "Write " .. param_field()()
    else
        tree:add(x328_proto.fields.parameter, tvb(5,4))
        pinfo.cols.info = "Query"
    end
end

prot_table = DissectorTable.get("udp.port")
prot_table:add(422, x328_proto)