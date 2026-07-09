# \TagEntityHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**create_tag_entity_handler**](TagEntityHandlerApi.md#create_tag_entity_handler) | **POST** /api/v1/tag-entities | 
[**delete_tag_entity_handler**](TagEntityHandlerApi.md#delete_tag_entity_handler) | **DELETE** /api/v1/tag-entities/{id} | 
[**get_tag_entity_handler**](TagEntityHandlerApi.md#get_tag_entity_handler) | **GET** /api/v1/tag-entities/{id} | 
[**tag_entity_list_handler**](TagEntityHandlerApi.md#tag_entity_list_handler) | **GET** /api/v1/tag-entities | 
[**tag_entity_list_search_handler**](TagEntityHandlerApi.md#tag_entity_list_search_handler) | **GET** /api/v1/tag-entities/search | 



## create_tag_entity_handler

> models::TagEntityModel create_tag_entity_handler(create_tag_entity_schema)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**create_tag_entity_schema** | [**CreateTagEntitySchema**](CreateTagEntitySchema.md) | Create tag entity | [required] |

### Return type

[**models::TagEntityModel**](TagEntityModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: application/json
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## delete_tag_entity_handler

> delete_tag_entity_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Tag entity id for deleting tag | [required] |

### Return type

 (empty response body)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_tag_entity_handler

> models::TagEntityModel get_tag_entity_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Tag entity id for fetching tag | [required] |

### Return type

[**models::TagEntityModel**](TagEntityModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## tag_entity_list_handler

> models::PagedResponseTagEntityModel tag_entity_list_handler(page, limit, order_by, desc, tag_id, entity_type)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**tag_id** | Option<**String**> | tag id for filtering tag entities |  |
**entity_type** | Option<**String**> | entity type for filtering tag entities |  |

### Return type

[**models::PagedResponseTagEntityModel**](PagedResponse_TagEntityModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## tag_entity_list_search_handler

> models::PagedResponseTagEntityModel tag_entity_list_search_handler(page, limit, order_by, desc, query)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**query** | Option<**String**> | search by tag name |  |

### Return type

[**models::PagedResponseTagEntityModel**](PagedResponse_TagEntityModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

