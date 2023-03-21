---
---
local x328_proto = Proto("x328", "X3.28 field bus")

x328_proto.fields.address = ProtoField.string("x328.address", "Address")
x328_proto.fields.parameter = ProtoField.string("x328.parameter", "Parameter")
x328_proto.fields.value = ProtoField.string("x328.value", "Value")
x328_proto.fields.bcc = ProtoField.uint8("x328.bcc", "BCC checksum")
x328_proto.fields.response = ProtoField.string("x328.response", "Node response")

local address_field = Field.new("x328.address")
local param_field = Field.new("x328.parameter")
local value_field = Field.new("x328.value")
local response = Field.new("x328.response")

function x328_proto.dissector(tvb, pinfo, tree)

    pinfo.cols.protocol= "X3.28"
    local tree = tree:add(x328_proto, tvb(), "X3.28 field bus")
    if pinfo.src == Address.ip("127.0.0.1") then
        dissect_master(tvb, pinfo, tree)
    else
        disecct_node(tvb, pinfo, tree)
    end
end

function dissect_master(tvb, pinfo, tree)
    tree:add(x328_proto.fields.address, tvb(2,2))
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
        tree:add(x328_proto.fields.bcc, tvb(10 + value_len + 1, 1))


        pinfo.cols.info = "Write addr " .. address_field()() .. " param " .. param_field()() .. " = " .. value_field()()
    else
        tree:add(x328_proto.fields.parameter, tvb(5,4))
        pinfo.cols.info = "Query addr " .. address_field()() .. " param " .. param_field()()
    end
end

function disecct_node(tvb, pinfo, tree)
    pinfo.cols.info = "Reply: "

    if tvb(0, 1):uint() == 6 then -- ACK
        tree:add(x328_proto.fields.response,"ACK")
    elseif tvb(0,1):uint() == 21 then -- NAK
        tree:add(x328_proto.fields.response, "NAK")
    else
        tree:add(x328_proto.fields.parameter, tvb(1,4))
        tree:add(x328_proto.fields.response, tvb(5, tvb:reported_len() - 5 - 2))
    end

    pinfo.cols.info = "Response: " .. response()()
end

prot_table = DissectorTable.get("udp.port")
prot_table:add(422, x328_proto)
